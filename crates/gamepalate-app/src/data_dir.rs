use std::path::PathBuf;

const DIR_ENV_VAR: &str = "GAMEPALATE_DATA_DIR";

pub fn resolve() -> Option<PathBuf> {
    std::env::var_os(DIR_ENV_VAR)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::data_dir().map(|dir| dir.join("gamepalate")))
}
