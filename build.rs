use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=../proto/game.proto");
    println!("cargo:rerun-if-changed=../omoba-core/src/generated/game.rs");

    if protoc_available() {
        compile_with_protoc();
    } else {
        let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR not set"));
        fs::copy(
            "../omoba-core/src/generated/game.rs",
            out_dir.join("game.rs"),
        )
        .expect("Failed to copy checked-in generated proto fallback");
        println!("cargo:warning=protoc not found; using omoba-core/src/generated/game.rs fallback");
    }
}

fn protoc_available() -> bool {
    if let Some(path) = env::var_os("PROTOC") {
        return Command::new(path).arg("--version").output().is_ok();
    }
    Command::new("protoc").arg("--version").output().is_ok()
}

fn compile_with_protoc() {
    #[cfg(feature = "grpc")]
    {
        tonic_build::configure()
            .build_server(true)
            .build_client(false)
            .compile_protos(&["../proto/game.proto"], &["../proto"])
            .expect("Failed to compile proto files");
    }

    #[cfg(feature = "kcp")]
    {
        prost_build::compile_protos(&["../proto/game.proto"], &["../proto"])
            .expect("Failed to compile proto files");
    }
}
