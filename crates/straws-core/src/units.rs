//! Length units for showing and typing dimensions, and AWG wire sizes.

/// Speed of light, m/s (as NEC uses).
pub const C: f64 = 299.792458e6;

/// Wavelength in metres at `mhz`.
pub fn wavelength(mhz: f64) -> f64 {
    C / (mhz * 1e6)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Length {
    #[default]
    Metres,
    Centimetres,
    Millimetres,
    Feet,
    Inches,
    /// Wavelengths at the current frequency.
    Wavelengths,
}

impl Length {
    pub const ALL: [Length; 6] = [
        Length::Metres,
        Length::Centimetres,
        Length::Millimetres,
        Length::Feet,
        Length::Inches,
        Length::Wavelengths,
    ];

    pub fn symbol(self) -> &'static str {
        match self {
            Length::Metres => "m",
            Length::Centimetres => "cm",
            Length::Millimetres => "mm",
            Length::Feet => "ft",
            Length::Inches => "in",
            Length::Wavelengths => "λ",
        }
    }

    /// Metres in one of this unit (wavelengths need the frequency).
    pub fn metres(self, mhz: f64) -> f64 {
        match self {
            Length::Metres => 1.0,
            Length::Centimetres => 0.01,
            Length::Millimetres => 0.001,
            Length::Feet => 0.3048,
            Length::Inches => 0.0254,
            Length::Wavelengths => wavelength(mhz),
        }
    }

    pub fn from_metres(self, m: f64, mhz: f64) -> f64 {
        m / self.metres(mhz)
    }

    pub fn to_metres(self, v: f64, mhz: f64) -> f64 {
        v * self.metres(mhz)
    }

    /// Decimal places that show about a tenth of a millimetre.
    pub fn decimals(self) -> usize {
        match self {
            Length::Metres | Length::Feet => 4,
            Length::Centimetres => 2,
            Length::Inches => 3,
            Length::Millimetres => 1,
            Length::Wavelengths => 5,
        }
    }
}

/// How wire sizes are shown and typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WireSize {
    /// Diameter in millimetres.
    #[default]
    Millimetres,
    /// Diameter in inches.
    Inches,
    /// American Wire Gauge.
    Awg,
}

impl WireSize {
    pub const ALL: [WireSize; 3] = [WireSize::Millimetres, WireSize::Inches, WireSize::Awg];

    pub fn label(self) -> &'static str {
        match self {
            WireSize::Millimetres => "diameter, mm",
            WireSize::Inches => "diameter, in",
            WireSize::Awg => "AWG",
        }
    }
}

/// Diameter in metres of AWG gauge `n` (0000 is -3).
pub fn awg_diameter(n: f64) -> f64 {
    0.005 * 0.0254 * 92f64.powf((36.0 - n) / 39.0)
}

/// The (fractional) AWG gauge of a diameter in metres.
pub fn awg_gauge(diameter: f64) -> f64 {
    36.0 - 39.0 * (diameter / (0.005 * 0.0254)).ln() / 92f64.ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert!((Length::Feet.to_metres(10.0, 14.0) - 3.048).abs() < 1e-12);
        assert!((Length::Wavelengths.from_metres(wavelength(7.0) / 2.0, 7.0) - 0.5).abs() < 1e-12);
        // AWG 14 is 1.628 mm; 4nec2's table says 1.6281.
        assert!((awg_diameter(14.0) * 1e3 - 1.628).abs() < 1e-3);
        assert!((awg_gauge(awg_diameter(12.0)) - 12.0).abs() < 1e-9);
    }

    #[test]
    fn every_unit_converts_back_exactly() {
        for u in Length::ALL {
            for m in [0.0, 1e-4, 0.123456789, 7.5, 1234.5] {
                let back = u.to_metres(u.from_metres(m, 14.2), 14.2);
                assert!((back - m).abs() <= 1e-12 * m.max(1.0), "{u:?} {m}");
            }
        }
    }
}
