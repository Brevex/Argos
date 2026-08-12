//! Build script for the shell.
//!
//! Besides Tauri's own code generation, this is where the Windows binary is
//! told to run as Administrator. Reading `\\.\PhysicalDrive0` needs that
//! privilege, and Windows only grants it to a process whose manifest asked for
//! it before the process started — there is no way to acquire it later.

/// The application manifest embedded in the Windows executable.
///
/// Two things are declared here. `requireAdministrator` makes Windows show the
/// consent prompt before the window exists, so the engine this process spawns
/// inherits the privileges a raw device needs. The Common Controls dependency
/// is Tauri's own default and has to be repeated, because supplying a manifest
/// replaces theirs rather than adding to it — without it the folder picker
/// draws with the pre-XP control set.
const WINDOWS_MANIFEST: &str = r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"
      />
    </dependentAssembly>
  </dependency>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="requireAdministrator" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#;

fn main() {
    let windows = tauri_build::WindowsAttributes::new().app_manifest(WINDOWS_MANIFEST);
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .unwrap_or_else(|err| panic!("the shell could not be built: {err}"));
}
