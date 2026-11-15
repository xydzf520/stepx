// We can use `std::process:Command` here because this is invoked within a build script,
// _not_ within the Warp binary (where it could cause a terminal to temporarily flash on
// Windows).
#![allow(clippy::disallowed_types)]
#![allow(semicolon_in_expressions_from_non_local_macros)]

use std::{
    env,
    path::PathBuf,
    process::{Command, Output},
};

use cfg_aliases::cfg_aliases;

fn main() {
    cfg_aliases! {
        macos: { target_os = "macos" },
        // We use winit on all platforms other than mac, where we have a custom
        // AppKit-based platform implementation.
        winit: { not(macos) },
        // We use wgpu for rendering on all platforms where we use winit, but
        // we can also use it on macOS, if enabled.
        wgpu: { any(winit, feature = "experimental-wgpu-renderer") },
        native: { not(target_family = "wasm") },
    }

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        bindgen_shader_types();
        compile_metal_shaders();
        compile_objc_lib();
    }
}

fn bindgen_shader_types() {
    let header_path = "src/platform/mac/rendering/metal/shaders/shader_types.h";
    println!("cargo:rerun-if-changed={header_path}");
    let bindings = bindgen::Builder::default()
        .header(header_path)
        .allowlist_type("vector_float2")
        .allowlist_type("Uniforms")
        .allowlist_type("PerRectUniforms")
        .allowlist_type("PerGlyphUniforms")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        // Disable intrinsic headers that define types containing 16-bit floats (`_Float16`,
        // `__m512h`, etc.) via preprocessor directive. `bindgen` doesn't know how to process
        // these types and panics at compile time. The types aren't used by our shader headers,
        // so suppressing the declarations is safe.
        //
        // - Xcode 15+: avx512fp16intrin.h, avx512vlfp16intrin.h
        // - Xcode 26+ (clang 21): amxavx512intrin.h, avx10_2convertintrin.h,
        //   avx10_2_512convertintrin.h
        //
        // TODO(charlespierce): Remove once https://github.com/rust-lang/rust-bindgen/issues/2500
        // is resolved.
        .clang_args([
            "-D__AVX512VLFP16INTRIN_H",
            "-D__AVX512FP16INTRIN_H",
            "-D__AMX_AVX512INTRIN_H",
            "-D__AVX10_2CONVERTINTRIN_H",
            "-D__AVX10_2_512CONVERTINTRIN_H",
            "-D__AVX10_2_512MINMAXINTRIN_H",
            "-D__AVX10_2_512NIINTRIN_H",
            "-D__AVX10_2_512SATCVTINTRIN_H",
        ])
        .generate()
        .unwrap_or_else(|_| panic!("unable to generate bindings for {header_path}"));

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("shader_types.rs"))
        .expect("Couldn't write shader type bindings!");
}

fn compile_metal_shaders() {
    let header_path = "src/platform/mac/rendering/metal/shaders/shader_types.h";
    let metal_path = "src/platform/mac/rendering/metal/shaders/shaders.metal";
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());

    let air_path = out_path.join("shaders.air");
    let air_path = air_path.to_str().unwrap();

    let lib_path = out_path.join("shaders.metallib");
    let lib_path = lib_path.to_str().unwrap();

    println!("cargo:rerun-if-changed={header_path}");
    println!("cargo:rerun-if-changed={metal_path}");
    println!("cargo:rerun-if-env-changed=FACLOUD_SKIP_METAL_SHADER_BUILD");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");

    if env::var_os("FACLOUD_SKIP_METAL_SHADER_BUILD").is_some() {
        println!("cargo:warning=FACLOUD_SKIP_METAL_SHADER_BUILD is set; writing placeholder shaders.metallib for local type-checking only");
        std::fs::write(lib_path, b"")
            .expect("error writing placeholder shaders.metallib for local type-checking");
        return;
    }

    let module_cache_arg = format!(
        "-fmodules-cache-path={}",
        out_path.join("metal-module-cache").display()
    );
    let min_version_arg = format!(
        "-mmacosx-version-min={}",
         macos_deployment_target()
    );

    match compile_metal_shaders_with_xcrun(
        metal_path,
        air_path,
        lib_path,
        &module_cache_arg,
        &min_version_arg,
    ) {
        Ok(()) => return,
        Err(xcrun_error) => match compile_metal_shaders_with_installed_toolchain(
            metal_path,
            lib_path,
            &module_cache_arg,
            &min_version_arg,
        ) {
            Ok(()) => {
                println!(
                    "cargo:warning=compiled metal shaders using installed MetalToolchain component because xcrun metallib was unavailable"
                );
            }
            Err(toolchain_error) => {
                panic!(
                    "error compiling metal shaders:\nlegacy xcrun path failed:\n{xcrun_error}\ninstalled MetalToolchain path failed:\n{toolchain_error}"
                );
            }
        },
    }
}

fn macos_deployment_target() -> String {
    env::var("MACOSX_DEPLOYMENT_TARGET")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "11.0".to_string())
}

fn compile_metal_shaders_with_xcrun(
    metal_path: &str,
    air_path: &str,
    lib_path: &str,
    module_cache_arg: &str,
    min_version_arg: &str,
) -> Result<(), String> {
    let mut compile_args = vec![
        "-sdk",
        "macosx",
        "metal",
        min_version_arg,
        module_cache_arg,
        "-c",
        metal_path,
        "-o",
        air_path,
    ];
    if cfg!(feature = "enable-metal-frame-capture") {
        compile_args.push("-frecord-sources");
        compile_args.push("-gline-tables-only");
    }
    let result = Command::new("xcrun")
        .args(&compile_args)
        .output()
        .map_err(|err| format!("error compiling metal shaders to .air: {err}"))?;
    if !result.status.success() {
        return Err(command_failure(
            "error compiling metal shaders to .air",
            result,
        ));
    }

    let result = Command::new("xcrun")
        .args(["-sdk", "macosx", "metallib", air_path, "-o", lib_path])
        .output()
        .map_err(|err| format!("error compiling metal shaders to .metallib: {err}"))?;
    if !result.status.success() {
        return Err(command_failure(
            "error compiling metal shaders to .metallib",
            result,
        ));
    }

    Ok(())
}

fn compile_metal_shaders_with_installed_toolchain(
    metal_path: &str,
    lib_path: &str,
    module_cache_arg: &str,
    min_version_arg: &str,
) -> Result<(), String> {
    let metal = installed_metal_toolchain_metal()?;
    let mut args = vec![min_version_arg, module_cache_arg];
    if cfg!(feature = "enable-metal-frame-capture") {
        args.push("-frecord-sources");
        args.push("-gline-tables-only");
    }
    args.extend([metal_path, "-o", lib_path]);

    let result = Command::new(metal)
        .args(args)
        .output()
        .map_err(|err| format!("error compiling metal shaders with MetalToolchain: {err}"))?;
    if !result.status.success() {
        return Err(command_failure(
            "error compiling metal shaders with MetalToolchain",
            result,
        ));
    }

    Ok(())
}

fn installed_metal_toolchain_metal() -> Result<PathBuf, String> {
    let output = Command::new("xcodebuild")
        .args(["-showComponent", "MetalToolchain"])
        .output()
        .map_err(|err| format!("error querying MetalToolchain component: {err}"))?;
    if !output.status.success() {
        return Err(command_failure(
            "error querying MetalToolchain component",
            output,
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let search_path = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Toolchain Search Path: "))
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| {
            format!("MetalToolchain component did not report a search path:\n{stdout}")
        })?;
    let metal = PathBuf::from(search_path).join("Metal.xctoolchain/usr/bin/metal");
    if !metal.is_file() {
        return Err(format!(
            "MetalToolchain component did not contain metal at {}",
            metal.display()
        ));
    }

    Ok(metal)
}

fn command_failure(action: &str, output: Output) -> String {
    format!(
        "{action}; status: {}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    )
}

fn compile_objc_lib() {
    println!("cargo:rustc-link-lib=framework=UserNotifications");
    println!("cargo:rustc-link-lib=framework=Carbon");
    println!("cargo:rustc-link-lib=framework=SystemConfiguration");
    println!("cargo:rustc-link-lib=framework=UniformTypeIdentifiers");
    println!("cargo:rustc-link-lib=framework=AVFoundation");
    println!("cargo:rustc-link-lib=framework=ServiceManagement");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/app.h");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/app.m");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/keycode.m");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/host_view.m");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/host_view.h");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/hotkey.h");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/hotkey.m");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/menus.h");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/menus.m");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/notifications/notifications.h");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/notifications/notifications.m");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/window.m");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/window_blur.m");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/window_blur.h");
    // Referenced from https://github.com/tonymillion/Reachability
    println!("cargo:rerun-if-changed=src/platform/mac/objc/reachability.h");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/reachability.m");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/alert.m");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/alert.h");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/fullscreen_queue.h");
    println!("cargo:rerun-if-changed=src/platform/mac/objc/fullscreen_queue.m");

    // Link against the clang_rt library so that the @available keyword
    // doesn't produce linker errors.
    //
    // See: https://github.com/alexcrichton/curl-rust/issues/279
    if let Some(path) = macos_link_search_path() {
        println!("cargo:rustc-link-lib=clang_rt.osx");
        println!("cargo:rustc-link-search={path}");
    }

    let min_version_arg = format!("-mmacosx-version-min={}", macos_deployment_target());
    cc::Build::new()
        .flag_if_supported(&min_version_arg)
        .file("src/platform/mac/objc/app.m")
        .file("src/platform/mac/objc/host_view.m")
        .file("src/platform/mac/objc/hotkey.m")
        .file("src/platform/mac/objc/reachability.m")
        .file("src/platform/mac/objc/keycode.m")
        .file("src/platform/mac/objc/menus.m")
        .file("src/platform/mac/objc/notifications/notifications.m")
        .file("src/platform/mac/objc/window.m")
        .file("src/platform/mac/objc/fullscreen_queue.m")
        .file("src/platform/mac/objc/window_blur.m")
        .file("src/platform/mac/objc/alert.m")
        .compile("warp_objc");
}

/// Determine the path containing the macOS standard libraries by querying
/// clang's library search paths.
fn macos_link_search_path() -> Option<String> {
    let output = Command::new("clang")
        .arg("--print-search-dirs")
        .output()
        .ok()?;
    if !output.status.success() {
        println!(
            "failed to run 'clang --print-search-dirs', continuing without a link search path"
        );
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("libraries: =") {
            let path = line.split('=').nth(1)?;
            return Some(format!("{path}/lib/darwin"));
        }
    }

    println!("failed to determine link search path, continuing without it");
    None
}
