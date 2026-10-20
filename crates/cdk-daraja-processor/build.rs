use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto = "proto/payment_processor.proto";
    println!("cargo:rerun-if-changed={proto}");
    let checksum = format!("{:x}", Sha256::digest(fs::read(proto)?));
    if checksum != "1b618a8f0bb0ecfc704e3b856697b524843bb944ae6bcde848698c06cd7b37ee" {
        return Err("CDK schema checksum differs from the pinned upstream contract".into());
    }
    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    let out = PathBuf::from(env::var("OUT_DIR")?);
    tonic_prost_build::configure()
        .file_descriptor_set_path(out.join("cdk_payment_processor.bin"))
        .compile_with_config(config, &[proto], &["proto"])?;
    Ok(())
}
