//! The matching network designer: networks that match the antenna at the
//! slider frequency to Z₀, their path on a Smith chart, and the matched
//! SWR across the sweep.

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, vec2};
use egui_plot::{Legend, Line, Plot, PlotPoints, VLine};
use straws_core::Complex64;
use straws_core::analysis::feed::swr;
use straws_core::analysis::matching::{
    Element, Part, input_impedance, l_networks, pi_networks, stub_matches, t_networks,
};

use crate::app::{Data, Settings};

use super::empty_note;
use super::smith::{gamma, grid, place, trace_colour};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    L,
    Pi,
    T,
    Stub,
}

pub struct MatchingView {
    kind: Kind,
    q: f64,
    shorted: bool,
    vf: f64,
    /// 0: exact values; 12 or 24: nearest standard values.
    series: usize,
    chosen: usize,
}

impl Default for MatchingView {
    fn default() -> Self {
        MatchingView {
            kind: Kind::L,
            q: 3.0,
            shorted: true,
            vf: 0.66,
            series: 0,
            chosen: 0,
        }
    }
}

/// Impedance after a fraction `t` of element `e` (for drawing its path).
fn partial(e: &Element, t: f64, z: Complex64, mhz: f64) -> Complex64 {
    match *e {
        Element::Series(p) => {
            Element::Series(Part::for_reactance(p.reactance(mhz) * t, mhz)).apply(z, mhz)
        }
        Element::Shunt(p) => {
            let b = -1.0 / p.reactance(mhz) * t;
            if b == 0.0 {
                z
            } else {
                Element::Shunt(Part::for_susceptance(b, mhz)).apply(z, mhz)
            }
        }
        Element::Line { length, z0, vf } => Element::Line {
            length: length * t,
            z0,
            vf,
        }
        .apply(z, mhz),
        Element::Stub { .. } => {
            // A stub adds susceptance: follow it from none to all.
            let full = e.apply(z, mhz);
            let b = (1.0 / full - 1.0 / z).im * t;
            1.0 / (1.0 / z + Complex64::new(0.0, b))
        }
    }
}

impl MatchingView {
    pub fn ui(&mut self, ui: &mut egui::Ui, data: &mut Data, settings: &Settings) {
        crate::views::feeds::feed_picker(ui, data);
        let Some(point) = data.shown_point() else {
            let note = data.waiting_note(
                "Open a model to design a matching network for it.",
                "Calculating…",
            );
            return empty_note(ui, note);
        };
        let Some(input) = point.feed(data.feed) else {
            return empty_note(ui, "This model has no voltage source to match.");
        };
        let (z, mhz, z0) = (input.impedance, point.mhz, settings.z0);

        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "Antenna at {mhz:.4} MHz: {:.2} {} j{:.2} Ω, SWR {:.2} on {z0:.0} Ω.",
                z.re,
                if z.im < 0.0 { "−" } else { "+" },
                z.im.abs(),
                swr(z, z0)
            ));
        });
        ui.horizontal_wrapped(|ui| {
            let before = (self.kind, self.q.to_bits(), self.shorted, self.vf.to_bits());
            ui.selectable_value(&mut self.kind, Kind::L, "L network")
                .on_hover_text("Two parts: the simplest match");
            ui.selectable_value(&mut self.kind, Kind::Pi, "Pi network")
                .on_hover_text("Shunt, series, shunt: choose the Q (bandwidth)");
            ui.selectable_value(&mut self.kind, Kind::T, "T network")
                .on_hover_text("Series, shunt, series: choose the Q (bandwidth)");
            ui.selectable_value(&mut self.kind, Kind::Stub, "Stub")
                .on_hover_text("A length of feedline, then a stub across it");
            ui.separator();
            match self.kind {
                Kind::Pi | Kind::T => {
                    ui.label("Q");
                    ui.add(
                        crate::views::num(&mut self.q)
                            .range(0.1..=100.0)
                            .speed(0.05),
                    )
                    .on_hover_text("Loaded Q: higher is narrower and filters harmonics more");
                }
                Kind::Stub => {
                    ui.checkbox(&mut self.shorted, "Shorted stub")
                        .on_hover_text("Shorted at its far end (off: open)");
                    ui.label("velocity factor");
                    ui.add(
                        crate::views::num(&mut self.vf)
                            .range(0.3..=1.0)
                            .speed(0.005),
                    )
                    .on_hover_text("Of the feedline and stub (0.66 solid PE coax, 0.82 foam)");
                }
                Kind::L => {}
            }
            if (self.kind, self.q.to_bits(), self.shorted, self.vf.to_bits()) != before {
                self.chosen = 0;
            }
            if self.kind != Kind::Stub {
                ui.separator();
                egui::ComboBox::from_id_salt("snap")
                    .selected_text(match self.series {
                        0 => "Exact values",
                        12 => "Nearest E12 parts",
                        _ => "Nearest E24 parts",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.series, 0, "Exact values");
                        ui.selectable_value(&mut self.series, 12, "Nearest E12 parts");
                        ui.selectable_value(&mut self.series, 24, "Nearest E24 parts");
                    });
            }
        });

        let networks = match self.kind {
            Kind::L => Ok(l_networks(z, z0, mhz)),
            Kind::Pi => pi_networks(z, z0, mhz, self.q),
            Kind::T => t_networks(z, z0, mhz, self.q),
            Kind::Stub => Ok(stub_matches(z, z0, self.vf, mhz, self.shorted)),
        };
        let networks: Vec<Vec<Element>> = match networks {
            Ok(n) => n
                .into_iter()
                .map(|n| {
                    if self.series == 0 {
                        n
                    } else {
                        n.iter().map(|e| e.snapped(self.series)).collect()
                    }
                })
                .collect(),
            Err(min_q) => {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!(
                        "This Q is too low for these impedances: use at least {:.2}.",
                        min_q + 0.01
                    ),
                );
                return;
            }
        };
        if networks.is_empty() {
            ui.label("No network of this kind matches this impedance.");
            return;
        }
        self.chosen = self.chosen.min(networks.len() - 1);
        ui.add_space(4.0);
        ui.label(RichText::new("From the antenna towards the radio:").weak());
        // The list scrolls, so the chart keeps its room in a short panel.
        egui::ScrollArea::vertical()
            .id_salt("solutions")
            .max_height(112.0)
            .show(ui, |ui| self.list(ui, &networks, z, z0, mhz));
        let network = &networks[self.chosen];
        self.draw(ui, data, network, z, z0, mhz);
    }

    fn list(
        &mut self,
        ui: &mut egui::Ui,
        networks: &[Vec<Element>],
        z: Complex64,
        z0: f64,
        mhz: f64,
    ) {
        for (k, n) in networks.iter().enumerate() {
            let zin = input_impedance(n, z, mhz);
            let text = format!(
                "{}.  {}   (SWR {:.2})",
                k + 1,
                n.iter()
                    .map(Element::describe)
                    .collect::<Vec<_>>()
                    .join(", then "),
                swr(zin, z0)
            );
            ui.radio_value(&mut self.chosen, k, text);
        }
    }

    fn draw(
        &mut self,
        ui: &mut egui::Ui,
        data: &Data,
        network: &[Element],
        z: Complex64,
        z0: f64,
        mhz: f64,
    ) {
        // Smith chart with the path, and the SWR across the sweep.
        let avail = ui.available_rect_before_wrap();
        let wide = avail.width() > avail.height();
        let (chart, plot) = if wide {
            let w = avail.height().min(avail.width() / 2.0);
            (
                Rect::from_min_size(avail.min, vec2(w, avail.height())),
                Rect::from_min_max(avail.min + vec2(w + 8.0, 0.0), avail.max),
            )
        } else {
            let h = avail.height() / 2.0;
            (
                Rect::from_min_size(avail.min, vec2(avail.width(), h)),
                Rect::from_min_max(avail.min + vec2(0.0, h + 4.0), avail.max),
            )
        };
        ui.allocate_rect(chart, Sense::hover());
        let (centre, radius) = grid(ui, chart);
        let painter = ui.painter_at(chart);
        let dark = ui.visuals().dark_mode;
        let colours = [
            Color32::from_rgb(86, 180, 233),
            Color32::from_rgb(0, 158, 115),
            Color32::from_rgb(204, 121, 167),
        ];
        let mut zc = z;
        let mut at: Pos2 = place(gamma(z, z0), centre, radius);
        painter.circle_filled(at, 5.0, trace_colour(dark));
        for (k, e) in network.iter().enumerate() {
            let pts: Vec<Pos2> = (0..=48)
                .map(|s| {
                    place(
                        gamma(partial(e, s as f64 / 48.0, zc, mhz), z0),
                        centre,
                        radius,
                    )
                })
                .collect();
            painter.add(egui::Shape::line(pts, Stroke::new(2.5, colours[k % 3])));
            zc = e.apply(zc, mhz);
            at = place(gamma(zc, z0), centre, radius);
            painter.circle_filled(at, 3.5, colours[k % 3]);
        }
        painter.circle_stroke(centre, 4.0, Stroke::new(1.5, ui.visuals().text_color()));

        let mut plot_ui = ui.new_child(egui::UiBuilder::new().max_rect(plot));
        if let Some(sweep) = data.display_sweep() {
            let before: PlotPoints = sweep
                .points()
                .filter_map(|p| {
                    p.feed(data.feed)
                        .map(|i| [p.mhz, swr(i.impedance, z0).min(20.0)])
                })
                .collect();
            let after: PlotPoints = sweep
                .points()
                .filter_map(|p| {
                    p.feed(data.feed).map(|i| {
                        [
                            p.mhz,
                            swr(input_impedance(network, i.impedance, p.mhz), z0).min(20.0),
                        ]
                    })
                })
                .collect();
            let marker = plot_ui.visuals().weak_text_color();
            Plot::new("matched swr")
                .include_y(1.0)
                .include_y(3.0)
                .y_axis_label("SWR")
                .x_axis_label("MHz")
                .allow_scroll(false)
                .legend(Legend::default())
                .show(&mut plot_ui, |pui| {
                    pui.line(
                        Line::new("antenna alone", before)
                            .color(trace_colour(dark))
                            .width(1.5),
                    );
                    pui.line(
                        Line::new("with this network", after)
                            .color(colours[0])
                            .width(2.0),
                    );
                    pui.vline(VLine::new("frequency", mhz).color(marker));
                });
        }
    }
}
