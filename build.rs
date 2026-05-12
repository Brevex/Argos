fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "windows")]
    embed_manifest::embed_manifest_file("resources/windows/Argos.manifest")?;

    tauri_build::build();
    Ok(())
}
