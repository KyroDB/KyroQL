fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=proto/kyroql.proto");

    // Only generate protobuf bindings when the gRPC transport feature is enabled.
    // This keeps embedded builds lean and avoids requiring `protoc` unless needed.
    if std::env::var_os("CARGO_FEATURE_TRANSPORT_GRPC").is_none() {
        return Ok(());
    }

    // Prefer a vendored protoc to avoid requiring a system installation.
    // This keeps `--features transport-grpc` and `--features server` builds reproducible.
    let protoc_path = protoc_bin_vendored::protoc_bin_path()
        .map_err(|e| format!("failed to locate vendored protoc: {e}"))?;
    std::env::set_var("PROTOC", protoc_path);

    // Only build proto if the proto file exists (allows consumers to omit proto/).
    if std::path::Path::new("proto/kyroql.proto").exists() {
        tonic_build::configure()
            .build_server(true)
            .build_client(true)
            .compile_protos(&["proto/kyroql.proto"], &["proto/"])?;
    }
    Ok(())
}
