use std::path::PathBuf;

pub fn resolve(option: Option<PathBuf>) -> Option<PathBuf> {
    option.or_else(|| dirs::data_dir().map(|dir| dir.join("gamepalate")))
}
