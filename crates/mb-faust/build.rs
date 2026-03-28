fn main() {
    let lib_dir = std::env::var("FAUST_LIB_DIR")
        .unwrap_or_else(|_| "/usr/local/lib".to_string());
    println!("cargo:rustc-link-search=native={lib_dir}");
    println!("cargo:rustc-link-lib=dylib=faust");
}
