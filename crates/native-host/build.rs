use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=../../icon.ico");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        let out_dir = match env::var("OUT_DIR") {
            Ok(d) => PathBuf::from(d),
            Err(_) => return,
        };
        let res_obj = out_dir.join("host_res.o");
        let manifest_dir = match env::var("CARGO_MANIFEST_DIR") {
            Ok(d) => PathBuf::from(d),
            Err(_) => return,
        };

        let pkg_version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "1.2.8".to_string());
        let mut ver_parts: Vec<&str> = pkg_version.split('.').collect();
        while ver_parts.len() < 4 {
            ver_parts.push("0");
        }
        let ver_comma = format!("{},{},{},{}", ver_parts[0], ver_parts[1], ver_parts[2], ver_parts[3]);
        let ver_str = format!("{}.{}.{}.{}", ver_parts[0], ver_parts[1], ver_parts[2], ver_parts[3]);

        let icon_path = manifest_dir.join("../../icon.ico");
        let icon_path_escaped = icon_path.to_string_lossy().replace('\\', "/");

        let generated_rc = format!(
r#"1 ICON "{}"

1 VERSIONINFO
FILEVERSION {}
PRODUCTVERSION {}
FILEOS 0x40004
FILETYPE 0x1
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "CompanyName", "Mahedi"
            VALUE "FileDescription", "Cosmo Downloader Native Host Bridge"
            VALUE "FileVersion", "{}"
            VALUE "InternalName", "cosmodownloader-host"
            VALUE "LegalCopyright", "Copyright (c) 2026 Mahedi"
            VALUE "OriginalFilename", "cosmodownloader-host.exe"
            VALUE "ProductName", "Cosmo Downloader"
            VALUE "ProductVersion", "{}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0409, 1200
    END
END
"#,
            icon_path_escaped, ver_comma, ver_comma, ver_str, ver_str
        );

        let gen_rc_file = out_dir.join("version.rc");
        let _ = std::fs::write(&gen_rc_file, generated_rc);

        let windres_bin = if Command::new("x86_64-w64-mingw32-windres").arg("--version").output().is_ok() {
            "x86_64-w64-mingw32-windres"
        } else {
            "windres"
        };

        let status = Command::new(windres_bin)
            .current_dir(&manifest_dir)
            .args(["-i", gen_rc_file.to_str().unwrap_or("app.rc"), "-o", res_obj.to_str().unwrap_or("host_res.o")])
            .status();

        if let Ok(s) = status {
            if s.success() {
                println!("cargo:rustc-link-arg={}", res_obj.display());
            }
        }
    }
}
