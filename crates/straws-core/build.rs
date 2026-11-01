// Compiles the vendored nec2c engine (see nec2c/README.straws.md).
//
// nec2c uses C99 complex numbers, which Microsoft's C compiler does not
// support, so on Windows it is built with clang-cl (Visual Studio's
// "C++ Clang Compiler for Windows" component). Linux and macOS use the
// system C compiler.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const SOURCES: &[&str] = &[
    "calculations.c",
    "fields.c",
    "geometry.c",
    "ground.c",
    "input.c",
    "main.c",
    "matrix.c",
    "misc.c",
    "network.c",
    "radiation.c",
    "shared.c",
    "somnec.c",
];

fn main() {
    println!("cargo:rerun-if-changed=nec2c");
    println!("cargo:rerun-if-env-changed=NEC_CLANG_CL");
    println!("cargo:rerun-if-env-changed=WASI_SDK_PATH");

    // The browser app (wasm32-unknown-unknown) has no C library: there the
    // engine is a separate WebAssembly worker, built for wasm32-wasip1.
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if arch == "wasm32" && os != "wasi" {
        return;
    }

    let src = Path::new("nec2c/src");
    let mut build = cc::Build::new();
    build
        .files(SOURCES.iter().map(|f| src.join(f)))
        .include(src)
        // nec2c's main() becomes a function the worker process calls.
        .define("main", "nec2c_main")
        .define("PACKAGE_STRING", "\"nec2c 1.3.1 (straws)\"")
        // The engine is always optimised, even in debug builds of the app:
        // an unoptimised solver is many times slower and hides real timings.
        .opt_level(2)
        .debug(false)
        // Vendored third-party code: its warnings are not ours to fix.
        .warnings(false);

    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        build
            .compiler(find_clang_cl())
            .include("nec2c/compat/win")
            .file("nec2c/compat/win/compat.c")
            .define("_CRT_SECURE_NO_WARNINGS", None)
            // Stops stdlib.h defining min()/max() macros, which clash with
            // nec2c's own min() function.
            .define("_CRT_DECLARE_NONSTDC_NAMES", "0")
            .define("_USE_MATH_DEFINES", None);
    }

    if os == "wasi" {
        // WebAssembly worker for the browser app: compiled with wasi-sdk's
        // clang against its C library (WASI_SDK_PATH, see web/README.md).
        let sdk = PathBuf::from(env::var_os("WASI_SDK_PATH").expect(
            "

Set WASI_SDK_PATH to a wasi-sdk folder to build the WebAssembly engine.

",
        ));
        let clang = sdk
            .join("bin")
            .join(if cfg!(windows) { "clang.exe" } else { "clang" });
        build
            .compiler(clang)
            .flag(format!(
                "--sysroot={}",
                sdk.join("share/wasi-sysroot").display()
            ))
            .flag("--target=wasm32-wasip1")
            // nec2c times its stages with times() and names temporary
            // files with getpid(); WASI emulates both.
            .define("_WASI_EMULATED_PROCESS_CLOCKS", None)
            .define("_WASI_EMULATED_GETPID", None);
        // Only these two libraries are taken from wasi-sdk (copied apart):
        // Rust brings its own C library, which must stay the one linked.
        let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
        let lib = sdk.join("share/wasi-sysroot/lib/wasm32-wasip1");
        for name in ["wasi-emulated-process-clocks", "wasi-emulated-getpid"] {
            let file = format!("lib{name}.a");
            std::fs::copy(lib.join(&file), out.join(&file)).expect("wasi-sdk's emulation library");
            println!("cargo:rustc-link-lib=static={name}");
        }
        println!("cargo:rustc-link-search=native={}", out.display());
    }

    build.compile("nec2c");
}

/// Finds clang-cl: $NEC_CLANG_CL, then PATH, then Visual Studio's LLVM folder.
fn find_clang_cl() -> PathBuf {
    if let Some(path) = env::var_os("NEC_CLANG_CL") {
        return PathBuf::from(path);
    }
    if Command::new("clang-cl").arg("--version").output().is_ok() {
        return PathBuf::from("clang-cl");
    }
    let vswhere = PathBuf::from(
        env::var_os("ProgramFiles(x86)").unwrap_or_else(|| r"C:\Program Files (x86)".into()),
    )
    .join(r"Microsoft Visual Studio\Installer\vswhere.exe");
    if let Ok(out) = Command::new(vswhere)
        .args(["-products", "*", "-latest", "-property", "installationPath"])
        .args([
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Llvm.Clang",
        ])
        .output()
    {
        let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !root.is_empty() {
            let clang = Path::new(&root).join(r"VC\Tools\Llvm\x64\bin\clang-cl.exe");
            if clang.exists() {
                return clang;
            }
        }
    }
    panic!(
        "\n\nCould not find clang-cl, which is needed to compile the nec2c engine on Windows.\n\
         Install it with the Visual Studio Installer: Modify > Individual components >\n\
         \"C++ Clang Compiler for Windows\". Or set NEC_CLANG_CL to the full path of clang-cl.exe.\n\n"
    );
}
