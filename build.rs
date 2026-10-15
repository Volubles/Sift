use std::{fs, io};

fn main() -> io::Result<()> {
    println!("cargo::rustc-check-cfg=cfg(sift_local_tests)");
    println!("cargo::rerun-if-changed=tests/unit");

    match fs::metadata("tests/unit") {
        Ok(metadata) if metadata.is_dir() => println!("cargo::rustc-cfg=sift_local_tests"),
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "The local unit-test path must be a directory",
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(())
}
