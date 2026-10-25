use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

pub fn helper(root: &Path) -> PathBuf {
    std::env::var_os("LAZYPENCIL_TEST_HELPER")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            root.join("xtask/target/debug").join(format!(
                "lazypencil-test-helper{}",
                std::env::consts::EXE_SUFFIX
            ))
        })
}

/// Install the native fixture with fixed arguments without a shell interpreter.
pub fn install(root: &Path, path: &Path, arguments: &[String]) -> io::Result<()> {
    let executable = helper(root);
    if path.exists() || path.is_symlink() || path.with_extension("fixture.json").exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Fixture output already exists",
        ));
    }
    if fs::hard_link(&executable, path).is_err() {
        let mut input = fs::File::open(&executable)?;
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)?;
        io::copy(&mut input, &mut output)?;
        fs::set_permissions(path, fs::metadata(&executable)?.permissions())?;
    }
    let sidecar = path.with_extension("fixture.json");
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(sidecar)?
        .write_all(&serde_json::to_vec(arguments).map_err(io::Error::other)?)
}
