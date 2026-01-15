pub fn normalize_object_name(path: &str) -> String {
    // 1. Get the part after the last slash (if any)
    let name = path.rsplit('/').next().unwrap_or(path);

    // 2. Split by dots. TERA paths are typically Package.Group.ObjectName
    if let Some(object_name) = name.rsplit('.').next() {
        let mut clean_name = object_name.to_string();

        // 3. Strip common class/suffixes that might differ between Modded and Vanilla files
        // _C is standard Unreal Engine class suffix
        // _dup, _lod0, etc. are common mesh/anim variants
        let suffixes = ["_C", "_dup", "_lod0", "_lod1", "_lod2", "_lod3"];
        for suffix in suffixes {
            if clean_name.ends_with(suffix) {
                clean_name.truncate(clean_name.len() - suffix.len());
            }
        }

        clean_name
    } else {
        // Fallback if no dots found
        name.to_string()
    }
}

pub fn incomplete_paths_equal(full: &str, incomplete: &str) -> bool {
    let full_name = normalize_object_name(full);
    let inc_name = normalize_object_name(incomplete);
    ascii_eq_ignore_case(&full_name, &inc_name)
}


pub fn ascii_eq_ignore_case(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes().zip(b.bytes()).all(|(x, y)| x.eq_ignore_ascii_case(&y))
}
