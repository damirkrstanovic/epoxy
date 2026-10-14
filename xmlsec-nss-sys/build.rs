use std::{env, path::PathBuf};

fn main() {
    // Use each distribution's headers and linker paths (including lib64 and
    // Debian multiarch). These are the same flags used by native C consumers.
    let xmlsec = pkg_config::Config::new()
        .atleast_version("1.2.39")
        .probe("xmlsec1-nss")
        .expect("XMLSec NSS development files >= 1.2.39 are required");
    let nss = pkg_config::Config::new()
        .probe("nss")
        .expect("NSS development files are required");
    let mut builder = bindgen::Builder::default()
        .header("src/lib.h")
        .blocklist_var("IPPORT_RESERVED")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .wrap_unsafe_ops(true);
    for library in [&xmlsec, &nss] {
        for include in &library.include_paths {
            builder = builder.clang_arg(format!("-I{}", include.display()));
        }
        for (name, value) in &library.defines {
            // The wrapper header already selects the NSS backend.
            if name == "XMLSEC_CRYPTO_NSS" {
                continue;
            }
            builder = builder.clang_arg(match value {
                Some(value) => format!("-D{name}={value}"),
                None => format!("-D{name}"),
            });
        }
    }
    let bindings = builder.generate().expect("Unable to generate bindings");
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
