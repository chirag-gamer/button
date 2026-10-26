fn main() {
    // tauri-build embeds the comctl32 v6 application manifest into bins
    // and cdylibs, but not into test executables. Tauri's window code
    // imports TaskDialogIndirect (comctl32 v6 only), so without the
    // manifest the loader binds comctl32 v5 and test binaries exit with
    // STATUS_ENTRYPOINT_NOT_FOUND before main runs. Embed the same
    // manifest tauri-build uses (windows-app-manifest.xml) into every
    // test target as well.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let manifest = r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
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
</assembly>
"#;
        let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
        let path = std::path::Path::new(&out_dir).join("test-app-manifest.xml");
        std::fs::write(&path, manifest).expect("write test manifest");
        println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}",
            path.display()
        );
    }
    tauri_build::build()
}
