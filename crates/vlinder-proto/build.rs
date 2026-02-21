fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/registry.proto")?;
    tonic_build::compile_protos("proto/state.proto")?;
    tonic_build::compile_protos("proto/harness.proto")?;
    tonic_build::compile_protos("proto/secret_store.proto")?;
    Ok(())
}
