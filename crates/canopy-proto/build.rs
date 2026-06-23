use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let source_proto = manifest_dir.join("proto").join("canopy.proto");

    let out_proto_dir = PathBuf::from(std::env::var("OUT_DIR")?).join("proto");
    std::fs::create_dir_all(&out_proto_dir)?;
    let out_proto = out_proto_dir.join("canopy.proto");
    std::fs::copy(&source_proto, &out_proto)?;

    println!("cargo:rerun-if-changed={}", source_proto.display());
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&[out_proto], &[out_proto_dir])?;
    Ok(())
}