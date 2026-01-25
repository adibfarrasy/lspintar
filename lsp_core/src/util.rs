use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use zip::ZipArchive;

pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

// Only find direct import match
pub fn naive_resolve_fqn(name: &str, imports: &[String]) -> Option<String> {
    if let Some(import) = imports.iter().find(|i| i.split('.').last() == Some(name)) {
        return Some(import.clone());
    }

    None
}

pub fn extract_jar_to_cache(
    jar_path: &str,
    cache_dir: PathBuf,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut hasher = DefaultHasher::new();
    jar_path.hash(&mut hasher);
    let jar_hash = hasher.finish();

    let extract_dir = cache_dir.join(jar_hash.to_string());

    if extract_dir.exists() {
        return Ok(extract_dir);
    }

    fs::create_dir_all(&extract_dir)?;

    let file = fs::File::open(jar_path)?;
    let mut archive = ZipArchive::new(file)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = extract_dir.join(file.name());

        if file.is_dir() {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent() {
                fs::create_dir_all(p)?;
            }
            let mut outfile = fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
        }
    }

    Ok(extract_dir)
}
