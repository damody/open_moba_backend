fn main() {
    println!("cargo:rerun-if-changed=../proto/game.proto");

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
