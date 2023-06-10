// Copyright (c) 2017-2021, The rav1e contributors. All rights reserved
//
// This source code is subject to the terms of the BSD 2 Clause License and
// the Alliance for Open Media Patent License 1.0. If the BSD 2 Clause License
// was not distributed with this source code in the LICENSE file, you can
// obtain it at www.aomedia.org/license/software. If the Alliance for Open
// Media Patent License 1.0 was not distributed with this source code in the
// PATENTS file, you can obtain it at www.aomedia.org/license/patent.

#![allow(clippy::print_literal)]
#![allow(clippy::unused_io_amount)]

#[allow(unused_imports)]
use std::env;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[allow(dead_code)]
fn rerun_dir<P: AsRef<Path>>(dir: P) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        println!("cargo:rerun-if-changed={}", path.to_string_lossy());

        if path.is_dir() {
            rerun_dir(path);
        }
    }
}

#[allow(dead_code)]
fn hash_changed(files: &[&str], out_dir: &str, config: &Path) -> Option<([u8; 8], PathBuf)> {
    use std::{collections::hash_map::DefaultHasher, hash::Hasher, io::Read};

    let mut hasher = DefaultHasher::new();

    let paths = files
        .iter()
        .map(Path::new)
        .chain(std::iter::once(config))
        .chain(std::iter::once(Path::new("build.rs")));

    for path in paths {
        if let Ok(mut f) = std::fs::File::open(path) {
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).unwrap();

            hasher.write(&buf);
        } else {
            panic!("Cannot open {}", path.display());
        }
    }

    let strip = env::var("STRIP").unwrap_or_else(|_| "strip".to_string());

    hasher.write(strip.as_bytes());

    let hash = hasher.finish().to_be_bytes();

    let hash_path = Path::new(&out_dir).join("asm.hash");

    if let Ok(old_hash) = std::fs::read(&hash_path) {
        if old_hash == hash {
            return None;
        }
    }

    Some((hash, hash_path))
}

#[cfg(feature = "asm")]
fn build_nasm_files() {
    use std::{fs::File, io::Write};
    let out_dir = env::var("OUT_DIR").unwrap();

    let dest_path = Path::new(&out_dir).join("config.asm");
    let mut config_file = File::create(&dest_path).unwrap();
    config_file
        .write(b"	%define private_prefix rav1e\n")
        .unwrap();
    config_file.write(b"	%define ARCH_X86_32 0\n").unwrap();
    config_file.write(b" %define ARCH_X86_64 1\n").unwrap();
    config_file.write(b"	%define PIC 1\n").unwrap();
    config_file.write(b" %define STACK_ALIGNMENT 16\n").unwrap();
    config_file.write(b" %define HAVE_AVX512ICL 1\n").unwrap();
    if env::var("CARGO_CFG_TARGET_VENDOR").unwrap() == "apple" {
        config_file.write(b" %define PREFIX 1\n").unwrap();
    }

    let asm_files = &["src/sad_plane/x86.asm"];

    if let Some((hash, hash_path)) = hash_changed(asm_files, &out_dir, &dest_path) {
        let mut config_include_arg = String::from("-I");
        config_include_arg.push_str(&out_dir);
        config_include_arg.push('/');
        let mut nasm = nasm_rs::Build::new();
        nasm.min_version(2, 14, 0);
        for file in asm_files {
            nasm.file(file);
        }
        nasm.flag(&config_include_arg);
        nasm.flag("-Isrc/");
        let obj = nasm.compile_objects().unwrap_or_else(|e| {
      println!("cargo:warning={e}");
      panic!("NASM build failed. Make sure you have nasm installed or disable the \"asm\" feature.\n\
        You can get NASM from https://nasm.us or your system's package manager.\n\nerror: {e}");
    });

        // cc is better at finding the correct archiver
        let mut cc = cc::Build::new();
        for o in obj {
            cc.object(o);
        }
        cc.compile("rav1easm");

        // Strip local symbols from the asm library since they
        // confuse the debugger.
        fn strip<P: AsRef<Path>>(obj: P) {
            let strip = env::var("STRIP").unwrap_or_else(|_| "strip".to_string());

            let mut cmd = std::process::Command::new(strip);

            cmd.arg("-x").arg(obj.as_ref());

            let _ = cmd.output();
        }

        strip(Path::new(&out_dir).join("librav1easm.a"));

        std::fs::write(hash_path, &hash[..]).unwrap();
    } else {
        println!("cargo:rustc-link-search={out_dir}");
    }
    println!("cargo:rustc-link-lib=static=rav1easm");
    rerun_dir("src/sad_plane");
    rerun_dir("src/ext/x86");
}

#[allow(unused_variables)]
fn main() {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    // let env = env::var("CARGO_CFG_TARGET_ENV").unwrap();

    #[cfg(feature = "asm")]
    {
        if arch == "x86_64" {
            println!("cargo:rustc-cfg={}", "nasm_x86_64");
            build_nasm_files()
        }
    }

    println!("cargo:rustc-env=PROFILE={}", env::var("PROFILE").unwrap());
    if let Ok(value) = env::var("CARGO_CFG_TARGET_FEATURE") {
        println!("cargo:rustc-env=CARGO_CFG_TARGET_FEATURE={value}");
    }
    println!(
        "cargo:rustc-env=CARGO_ENCODED_RUSTFLAGS={}",
        env::var("CARGO_ENCODED_RUSTFLAGS").unwrap()
    );
}
