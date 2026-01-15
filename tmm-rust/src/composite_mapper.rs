use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use indexmap::IndexMap;
use crate::utils::incomplete_paths_equal;

const KEY1: [usize; 16] = [12, 6, 9, 4, 3, 14, 1, 10, 13, 2, 7, 15, 0, 8, 5, 11];
const KEY2: &[u8] = b"GeneratePackageMapper";

#[derive(Default, Clone)]
pub struct CompositeEntry {
    pub filename: String,
    pub object_path: String,
    pub composite_name: String,
    pub offset: usize,
    pub size: usize,
}

#[derive(Default, Clone)]
pub struct CompositeMapperFile {
    pub source_path: PathBuf,
    pub source_size: usize,
    pub composite_map: IndexMap<String, CompositeEntry>,
    pub dirty: bool,
    pub cached_map: String,
    pub plaintext: String,
}

impl CompositeMapperFile {
    pub fn new(source_path: PathBuf) -> std::io::Result<Self> {
        let mut mapper = Self {
            source_path,
            ..Default::default()
        };
        mapper.reload()?;
        Ok(mapper)
    }

    pub fn reload(&mut self) -> std::io::Result<()> {
        let encrypted = fs::read(&self.source_path)?;
        let decrypted = Self::decrypt_mapper(&encrypted)?;

        self.source_size = decrypted.len();
        self.plaintext = decrypted.clone();
        self.composite_map.clear();

        self.parse_entries_with_offsets(&decrypted);

        Ok(())
    }

    pub fn save(&self, dest: &Path) -> std::io::Result<()> {
        // Generate fresh content from the map structure
        let mut plaintext = String::new();
        Self::serialize_composite_map_to_string(&self.composite_map, &mut plaintext, 0);
        
        let encrypted = Self::encrypt_mapper(plaintext.as_bytes());
        fs::write(dest, encrypted)
    }

    pub fn get_entry_by_incomplete_object_path(
        &self,
        path: &str,
        output: &mut CompositeEntry,
    ) -> bool {
        let matches: Vec<&CompositeEntry> = self
            .composite_map
            .values()
            .filter(|e| incomplete_paths_equal(&e.object_path, path))
            .collect();

        if matches.len() != 1 {
            return false;
        }

        *output = matches[0].clone();
        true
    }


    pub fn remove_entry(&mut self, entry: &CompositeEntry) -> bool {
        let removed = self.composite_map.shift_remove(&entry.composite_name).is_some();
        if removed {
            self.cached_map.clear();
        }
        removed
    }

    pub fn apply_patch(
        &mut self,
        composite_name: &str,
        new_filename: &str,
        new_offset: usize,
        new_size: usize,
    ) -> Result<()> {
        let entry = self
            .composite_map
            .get_mut(composite_name)
            .context("Composite entry not found")?;
        
        entry.filename = new_filename.to_string();
        entry.offset = new_offset;
        entry.size = new_size;

        self.dirty = true;
        Ok(())
    }

    fn parse_entries_with_offsets(&mut self, data: &str) {
        
        let mut cursor = 0;

        while let Some(q) = data[cursor..].find('?') {
            let file_start = cursor;
            let file_end = cursor + q;
            let filename = &data[file_start..file_end];
            cursor = file_end + 1;

            let excl = match data[cursor..].find('!') {
                Some(p) => cursor + p,
                None => break,
            };

            let block = &data[cursor..excl];
            let mut pos = 0;

            while let Some(sep) = block[pos..].find(",|") {
                let entry_start = pos;
                let entry_end = pos + sep;
                let slice = &block[entry_start..entry_end];
                pos += sep + 2;

                let mut it = slice.split(',');

                let object_path = it.next().unwrap();
                let composite_name = it.next().unwrap();

                let offset_str = it.next().unwrap();
                let size_str = it.next().unwrap();

                let entry = CompositeEntry {
                    filename: filename.to_string(),
                    object_path: object_path.to_string(),
                    composite_name: composite_name.to_string(),
                    offset: offset_str.parse().unwrap_or(0),
                    size: size_str.parse().unwrap_or(0),
                };

                self.composite_map.insert(entry.composite_name.clone(), entry);
            }

            cursor = excl + 1;
        }
    }

    pub fn serialize_composite_map_to_string(
        composite_map: &IndexMap<String, CompositeEntry>,
        output: &mut String,
        _source_size: usize,
    ) {
        output.clear();

        let mut by_file: IndexMap<&str, Vec<&CompositeEntry>> = IndexMap::new();

        for entry in composite_map.values() {
            by_file
                .entry(entry.filename.as_str())
                .or_default()
                .push(entry);
        }

        // Sort by offset, not composite_name. The game engine relies on offset order.
        for entries in by_file.values_mut() {
            entries.sort_by(|a, b| a.offset.cmp(&b.offset));
        }

        for (filename, entries) in by_file {
            if filename.is_empty() {
                continue; // Skip entries with empty filenames to prevent invalid map blocks
            }
            
            output.push_str(filename);
            output.push('?');

            for e in entries {
                output.push_str(&e.object_path);
                output.push(',');
                output.push_str(&e.composite_name);
                output.push(',');
                output.push_str(&e.offset.to_string());
                output.push(',');
                output.push_str(&e.size.to_string());
                output.push_str(",|");
            }

            output.push('!');
        }
    }

    fn encrypt_mapper(input: &[u8]) -> Vec<u8> {
        let size = input.len();
        let mut encrypted = input.to_vec();

        // XOR stage
        for i in 0..size {
            encrypted[i] ^= KEY2[i % KEY2.len()];
        }

        // Swap stage
        if size > 2 {
            let mut a = 1usize;
            let mut b = size - 1;
            let count = (size / 2 + 1) / 2;
            for _ in 0..count {
                encrypted.swap(a, b);
                a += 2;
                b = b.saturating_sub(2);
            }
        }
        // Block permutation
        let mut tmp = [0u8; 16];
        let mut offset = 0;
        while offset + 16 <= size {
            tmp.copy_from_slice(&encrypted[offset..offset + 16]);
            for i in 0..16 {
                encrypted[offset + i] = tmp[KEY1[i]];
            }
            offset += 16;
        }

        encrypted
    }

        fn decrypt_mapper(input: &[u8]) -> std::io::Result<String> {
            let size = input.len();
            let mut decrypted = input.to_vec();

            // Block permutation inverse
            let mut tmp = [0u8; 16];
            let mut offset = 0;
            while offset + 16 <= size {
                tmp.copy_from_slice(&decrypted[offset..offset + 16]);
                for i in 0..16 {
                    decrypted[offset + KEY1[i]] = tmp[i];
                }
                offset += 16;
            }

            // Swap inverse
            if size > 2 {
                let mut a = 1usize;
                let mut b = size - 1;
                let count = (size / 2 + 1) / 2;
                for _ in 0..count {
                    decrypted.swap(a, b);
                    a += 2;
                    b = b.saturating_sub(2);
                }
            }

            // XOR inverse
            for i in 0..size {
                decrypted[i] ^= KEY2[i % KEY2.len()];
            }

            Ok(String::from_utf8_lossy(&decrypted).into_owned())
        }
}
