//! The package identity that puts JustTools in the Windows 11 context menu.
//!
//! Windows 11 only shows entries in its top-level menu for apps that have a
//! package identity. JustTools stays a portable folder, so it registers a
//! "sparse" package: a manifest plus logos that point back at the installed
//! folder as their external location.

use std::fs;
use std::path::Path;

use justtools_menu as menu;

use crate::error::{ToolError, ToolResult};

pub const NAME: &str = "JustGains.JustTools";
/// The subject used when no signing certificate dictates one.
pub const DEFAULT_PUBLISHER: &str = "CN=JustGains";
pub const MANIFEST: &str = "AppxManifest.xml";

const LOGOS: [(&str, &[u8]); 3] = [
    (
        "logo-44.png",
        include_bytes!("../../explorer/icons/logo-44.png"),
    ),
    (
        "logo-150.png",
        include_bytes!("../../explorer/icons/logo-150.png"),
    ),
    (
        "logo-store.png",
        include_bytes!("../../explorer/icons/logo-store.png"),
    ),
];

fn architecture() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else {
        "x64"
    }
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Every Explorer item type the menu attaches to.
pub fn item_types() -> Vec<String> {
    let mut types = vec!["Directory".to_owned(), r"Directory\Background".to_owned()];
    types.extend(
        menu::extensions()
            .into_iter()
            .map(|extension| format!(".{extension}")),
    );
    types
}

/// The package manifest for this build, signed or registered as `publisher`.
pub fn manifest(publisher: &str) -> String {
    let class = menu::CLASS_ID;
    let version = format!("{}.0", env!("CARGO_PKG_VERSION"));
    let verbs: String = item_types()
        .iter()
        .map(|kind| {
            format!(
                "            <desktop5:ItemType Type=\"{}\">\n              <desktop5:Verb Id=\"JustTools\" Clsid=\"{class}\" />\n            </desktop5:ItemType>\n",
                escape(kind)
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Package
  xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:uap10="http://schemas.microsoft.com/appx/manifest/uap/windows10/10"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  xmlns:desktop4="http://schemas.microsoft.com/appx/manifest/desktop/windows10/4"
  xmlns:desktop5="http://schemas.microsoft.com/appx/manifest/desktop/windows10/5"
  xmlns:com="http://schemas.microsoft.com/appx/manifest/com/windows10"
  IgnorableNamespaces="uap uap10 rescap desktop4 desktop5 com">
  <Identity Name="{NAME}" Publisher="{publisher}" Version="{version}" ProcessorArchitecture="{architecture}" />
  <Properties>
    <DisplayName>JustTools</DisplayName>
    <PublisherDisplayName>JustGains</PublisherDisplayName>
    <Logo>Assets\logo-store.png</Logo>
    <uap10:AllowExternalContent>true</uap10:AllowExternalContent>
  </Properties>
  <Resources>
    <Resource Language="en-us" />
  </Resources>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.22000.0" MaxVersionTested="10.0.26100.0" />
  </Dependencies>
  <Capabilities>
    <rescap:Capability Name="runFullTrust" />
    <rescap:Capability Name="unvirtualizedResources" />
  </Capabilities>
  <Applications>
    <Application Id="JustTools" Executable="just.exe" uap10:TrustLevel="mediumIL" uap10:RuntimeBehavior="win32App">
      <uap:VisualElements AppListEntry="none" DisplayName="JustTools" Description="JustTools File Explorer context menu" BackgroundColor="transparent" Square150x150Logo="Assets\logo-150.png" Square44x44Logo="Assets\logo-44.png" />
      <Extensions>
        <desktop4:Extension Category="windows.fileExplorerContextMenus">
          <desktop4:FileExplorerContextMenus>
{verbs}          </desktop4:FileExplorerContextMenus>
        </desktop4:Extension>
        <com:Extension Category="windows.comServer">
          <com:ComServer>
            <com:SurrogateServer DisplayName="JustTools context menu">
              <com:Class Id="{class}" Path="{library}" ThreadingModel="STA" />
            </com:SurrogateServer>
          </com:ComServer>
        </com:Extension>
      </Extensions>
    </Application>
  </Applications>
</Package>
"#,
        publisher = escape(publisher),
        architecture = architecture(),
        library = menu::SHELL_LIBRARY,
    )
}

/// Write the manifest and its logos as the layout `makeappx pack` and
/// `Add-AppxPackage -Register` both read.
pub fn write_layout(directory: &Path, publisher: &str) -> ToolResult {
    let failed = |path: &Path, error: std::io::Error| {
        ToolError::new("just", format!("cannot write {}: {error}", path.display()))
    };
    let assets = directory.join("Assets");
    fs::create_dir_all(&assets).map_err(|error| failed(&assets, error))?;
    for (name, bytes) in LOGOS {
        let path = assets.join(name);
        fs::write(&path, bytes).map_err(|error| failed(&path, error))?;
    }
    let path = directory.join(MANIFEST);
    fs::write(&path, manifest(publisher)).map_err(|error| failed(&path, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_registers_every_item_type_for_the_shared_class() {
        let manifest = manifest("CN=Example & Co");
        assert!(manifest.contains(r#"Publisher="CN=Example &amp; Co""#));
        assert!(manifest.contains(&format!(r#"Version="{}.0""#, env!("CARGO_PKG_VERSION"))));
        assert!(manifest.contains(r#"<desktop5:ItemType Type=".mp4">"#));
        assert!(manifest.contains(r#"<desktop5:ItemType Type="Directory\Background">"#));
        assert!(!manifest.contains(r#"Type=".ts""#));
        assert_eq!(
            manifest.matches(menu::CLASS_ID).count(),
            item_types().len() + 1
        );
        assert!(manifest.contains(r#"Path="justtools_shell.dll""#));
    }

    #[test]
    fn layout_holds_the_manifest_and_every_logo_it_names() {
        let directory = tempfile::tempdir().unwrap();
        write_layout(directory.path(), DEFAULT_PUBLISHER).unwrap();
        let manifest = fs::read_to_string(directory.path().join(MANIFEST)).unwrap();
        for (name, _) in LOGOS {
            assert!(manifest.contains(&format!(r"Assets\{name}")));
            let logo = fs::read(directory.path().join("Assets").join(name)).unwrap();
            assert!(logo.starts_with(b"\x89PNG"));
        }
    }
}
