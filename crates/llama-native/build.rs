//! llama.cpp (downloaded at a pinned commit, patched, built with cmake) and
//! the native/shim.cpp C API over it, laid beside the dependent workspace's
//! binaries. A dependent adds `$ORIGIN` to its binaries' runpath itself
//! (linker arguments do not reach them from here).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// llama.cpp b11379: the clef arch is PR #29831 (b11371).
const LLAMA_COMMIT: &str = "1537a0a8b2f8711d840878b0a0677ab2213c882c";
const LLAMA_BUILD_NUMBER: &str = "11379";
/// GitHub's source archive of `LLAMA_COMMIT` (37,840,167 bytes).
const LLAMA_SHA256: &str = "372ba5251b9e1a88cd4dc9127686bf9fdd9fb5820884648e561f5c8c07d4ba34";

fn main() {
    llama();
}

/// llama.cpp's source in OUT_DIR, patched with native/*.patch; extracted again
/// when the patches change.
fn llama_source(out: &Path) -> PathBuf {
    let src = out.join(format!("llama.cpp-{LLAMA_COMMIT}"));
    let mut patches: Vec<PathBuf> = fs::read_dir("native")
        .expect("native/ is readable")
        .map(|entry| entry.expect("native/ entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "patch"))
        .collect();
    patches.sort();
    let stamp: String = patches
        .iter()
        .map(|patch| fs::read_to_string(patch).expect("patch is readable"))
        .collect();
    let marker = src.join(".llama-native-patches");
    if fs::read_to_string(&marker).ok() == Some(stamp.clone()) {
        return src;
    }
    let tarball = out.join("llama.cpp.tar.gz");
    if sha256_hex(&tarball).as_deref() != Some(LLAMA_SHA256) {
        if let Ok(local) = env::var("III_LLAMA_CPP_TARBALL") {
            fs::copy(&local, &tarball).unwrap_or_else(|e| panic!("copying {local}: {e}"));
        } else {
            let url =
                format!("https://github.com/ggml-org/llama.cpp/archive/{LLAMA_COMMIT}.tar.gz");
            let status = Command::new("curl")
                .args([
                    "-fsSL",
                    "--retry",
                    "3",
                    "--retry-all-errors",
                    "--retry-delay",
                    "5",
                    "-o",
                ])
                .arg(&tarball)
                .arg(&url)
                .status()
                .expect("curl must be on PATH (or set III_LLAMA_CPP_TARBALL to a local copy)");
            assert!(status.success(), "downloading {url} failed");
        }
        assert_eq!(
            sha256_hex(&tarball).unwrap_or_default(),
            LLAMA_SHA256,
            "llama.cpp archive digest mismatch: refusing to build unverified source"
        );
    }
    let _ = fs::remove_dir_all(&src);
    // cmake-rs's build tree: re-extracted sources carry the archive's old
    // mtimes, so make would keep objects built from the previous patch set.
    let _ = fs::remove_dir_all(out.join("build"));
    run(Path::new("tar"), &["-xzf", "llama.cpp.tar.gz"], out);
    for patch in &patches {
        let patch = fs::canonicalize(patch).expect("patch path");
        // -F0: no fuzz, every context line must still match.
        eprintln!("applying {}", patch.display());
        run(
            Path::new("patch"),
            &[
                "-p1",
                "-N",
                "-F0",
                "-s",
                "-i",
                patch.to_str().expect("UTF-8 path"),
            ],
            &src,
        );
    }
    fs::write(&marker, stamp).expect("writing the patch marker");
    src
}

fn sha256_hex(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    Some(format!("{:x}", Sha256::digest(fs::read(path).ok()?)))
}

/// Build llama.cpp and native/shim.cpp, link them, and lay the shared
/// libraries and backend modules beside the binaries (Linux x86_64).
fn llama() {
    println!("cargo:rerun-if-changed=native");
    println!("cargo:rerun-if-env-changed=III_LLAMA_CPP_TARBALL");
    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let src = llama_source(&out);
    let os = env::var("CARGO_CFG_TARGET_OS").expect("cargo sets the target OS");
    let arch = env::var("CARGO_CFG_TARGET_ARCH").expect("cargo sets the target arch");
    // Linux x86_64 loads its backends at runtime: the CPU variants and the
    // Vulkan module beside the binary, so one build runs on any x86-64 CPU,
    // with or without a GPU. Elsewhere they are linked in statically: Metal
    // on macOS, the CPU otherwise (crates/llama-runtime's target table).
    let dynamic = os == "linux" && arch == "x86_64";
    let backends = out.join("backends");
    let mut cmake = cmake::Config::new(&src);
    cmake
        .profile("Release")
        .define("LLAMA_BUILD_COMMON", "OFF")
        .define("LLAMA_BUILD_TESTS", "OFF")
        .define("LLAMA_BUILD_EXAMPLES", "OFF")
        .define("LLAMA_BUILD_TOOLS", "OFF")
        .define("LLAMA_BUILD_SERVER", "OFF")
        .define("LLAMA_BUILD_APP", "OFF")
        .define("LLAMA_BUILD_NUMBER", LLAMA_BUILD_NUMBER)
        .define("LLAMA_BUILD_COMMIT", &LLAMA_COMMIT[..8])
        .define("GGML_NATIVE", "OFF")
        .define("GGML_OPENMP", "OFF")
        .define("BUILD_SHARED_LIBS", if dynamic { "ON" } else { "OFF" });
    if dynamic {
        fs::create_dir_all(&backends).expect("creating the backends directory");
        cmake
            .define("GGML_BACKEND_DL", "ON")
            .define("GGML_CPU_ALL_VARIANTS", "ON")
            .define("GGML_VULKAN", "ON")
            .define("GGML_BACKEND_DIR", &backends);
    }
    if os == "macos" {
        // Metal (and its embedded shader library) is llama.cpp's Apple default.
        cmake.define("GGML_BLAS", "OFF");
    }
    if os == "linux" && arch == "aarch64" {
        cmake.define("GGML_CPU_ARM_ARCH", "armv8-a");
    }
    let installed = cmake.build();

    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file("native/shim.cpp")
        .include(src.join("include"))
        .include(src.join("src"))
        .include(src.join("ggml/include"))
        .compile("llama-native");

    // CMake's GNUInstallDirs picks lib64 on some distributions (Fedora).
    let lib = ["lib", "lib64"]
        .map(|dir| installed.join(dir))
        .into_iter()
        .find(|dir| dir.is_dir())
        .expect("llama.cpp installs its libraries under lib or lib64");
    println!("cargo:rustc-link-search=native={}", lib.display());
    if dynamic {
        for name in ["llama", "ggml", "ggml-base"] {
            println!("cargo:rustc-link-lib=dylib={name}");
        }
        // Tests run from deps/ without the modules: they load them from here.
        println!(
            "cargo:rustc-env=LLAMA_NATIVE_BACKENDS_DIR={}",
            backends.display()
        );
        lay_beside_binaries(&out, &lib, &backends);
        return;
    }
    // Static archives in dependency order for single-pass linkers: llama,
    // ggml, its backends, ggml-base.
    let mut archives: Vec<String> = files(&lib)
        .iter()
        .filter_map(|path| {
            path.file_name()?
                .to_str()?
                .strip_prefix("lib")?
                .strip_suffix(".a")
        })
        .map(str::to_owned)
        .collect();
    archives.sort_by_key(|name| match name.as_str() {
        "llama" => 0,
        "ggml" => 1,
        "ggml-base" => 3,
        _ => 2,
    });
    for name in archives {
        println!("cargo:rustc-link-lib=static={name}");
    }
    if os == "macos" {
        for framework in ["Foundation", "Metal", "MetalKit", "Accelerate"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
        // ggml-metal's `@available` checks need compiler-rt (llama-cpp-sys-2).
        if let Some(dir) = clang_runtime_dir() {
            println!("cargo:rustc-link-search={dir}");
            println!("cargo:rustc-link-lib=clang_rt.osx");
        }
    }
}

/// `clang --print-search-dirs`'s `lib/darwin`, as llama-cpp-sys-2 finds it.
fn clang_runtime_dir() -> Option<String> {
    let output = Command::new("clang")
        .arg("--print-search-dirs")
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().find(|line| line.contains("libraries: ="))?;
    Some(format!("{}/lib/darwin", line.split('=').nth(1)?))
}

/// Copy the backend modules into the profile directory (the published layout)
/// and the libraries' SONAME files (`libllama.so.0`) there and into deps/ and
/// examples/, where tests and examples run (crates/llama-runtime/build.rs).
fn lay_beside_binaries(out: &Path, lib: &Path, backends: &Path) {
    // OUT_DIR is <profile>/build/<crate>-<hash>/out.
    let profile_dir = out
        .ancestors()
        .nth(3)
        .expect("OUT_DIR lies under the profile directory")
        .to_path_buf();
    let library_dirs = [
        profile_dir.clone(),
        profile_dir.join("deps"),
        profile_dir.join("examples"),
    ];
    // Release copies are what ships: strip them like the binary (`strip = true`).
    let strip = env::var("PROFILE").as_deref() == Ok("release");
    for module in files(backends) {
        copy_into(&module, &profile_dir, strip);
    }
    for library in files(lib) {
        let name = library
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let soname = name.split_once(".so.").is_some_and(|(_, version)| {
            !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit())
        });
        if soname {
            for dir in library_dirs.iter().filter(|dir| dir.is_dir()) {
                copy_into(&library, dir, strip);
            }
        }
    }
}

fn files(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
        .map(|entry| entry.expect("directory entry is readable").path())
        .collect()
}

/// Copy the file's contents (following symlinks) under its own name; `strip`
/// removes the symbols the loader does not need (release builds).
fn copy_into(path: &Path, dir: &Path, strip: bool) {
    let destination = dir.join(path.file_name().expect("file has a name"));
    let _ = fs::remove_file(&destination);
    fs::copy(path, &destination).unwrap_or_else(|error| {
        panic!(
            "copy {} to {}: {error}",
            path.display(),
            destination.display()
        )
    });
    if strip
        && !Command::new("strip")
            .arg("--strip-unneeded")
            .arg(&destination)
            .status()
            .is_ok_and(|status| status.success())
    {
        println!("cargo:warning=could not strip {}", destination.display());
    }
}

fn run(command: &Path, args: &[&str], cwd: &Path) {
    let status = Command::new(command)
        .args(args)
        .current_dir(cwd)
        .status()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", command.display()));
    if !status.success() {
        panic!("{} exited with {status}", command.display());
    }
}
