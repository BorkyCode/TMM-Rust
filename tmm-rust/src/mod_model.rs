use anyhow::Result;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::default::Default;
use std::io::{Read, Seek, SeekFrom, Write};

#[derive(Default, Clone, PartialEq, Eq)]
pub struct CompositePackage {
    pub object_path: String,
    pub offset: usize,
    pub size: usize,
    pub file_version: u16,
    pub licensee_version: u16,
}

#[derive(Default, Clone, PartialEq, Eq)]
pub struct TfcPackage {
    pub offset: i32,
    pub size: i32,
    pub idx: i32,
    pub idx_offset: i32,
}

#[derive(Default, Clone, PartialEq)]
pub struct ModFile {
    pub region_lock: bool,
    pub mod_file_version: i32,
    pub mod_name: String,
    pub container: String,
    pub mod_author: String,
    pub packages: Vec<CompositePackage>,
    pub tfc_packages: Vec<TfcPackage>,
}

#[derive(Default, Clone, PartialEq)]
pub struct ModEntry {
    pub file: String,
    pub enabled: bool,
    pub mod_file: ModFile,
}

#[derive(Default, Clone, PartialEq)]
pub struct GameConfigFile {
    pub mods: Vec<ModEntry>,
}

const PACKAGE_MAGIC: u32 = 0x9E2A83C1;
const MAX_STRLEN: usize = 1024;

pub fn read_string<R: Read>(r: &mut R) -> Result<String> {
    let mut size: i32 = r.read_i32::<LittleEndian>()?;
    if size == 0 {
        return Ok(String::new());
    }
    let is_wide = size < 0;
    if is_wide {
        size = -size;
    }
    if size as usize > MAX_STRLEN {
        return Err(anyhow::anyhow!("String too long"));
    }
    let byte_len = size as usize * if is_wide { 2 } else { 1 };
    let mut buf = vec![0u8; byte_len];
    r.read_exact(&mut buf)?;
    let mut out = if is_wide {
        let wide: Vec<u16> = buf.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&wide).to_string()
    } else {
        String::from_utf8_lossy(&buf).to_string()
    };
    if out.ends_with('\0') {
        out.pop();
    }
    Ok(out)
}

pub fn write_string<W: Write>(w: &mut W, s: &str) -> Result<()> {
    let is_ansi = s.is_ascii();
    if is_ansi {
        let size = s.len() as i32;
        w.write_i32::<LittleEndian>(size)?;
        w.write_all(s.as_bytes())?;
    } else {
        let wide: Vec<u16> = s.encode_utf16().collect();
        let size = -(wide.len() as i32);
        w.write_i32::<LittleEndian>(size)?;
        for &c in &wide {
            w.write_u16::<LittleEndian>(c)?;
        }
    }
    Ok(())
}

pub fn read_mod_file<R: Read + Seek>(s: &mut R, m: &mut ModFile) -> Result<()> {
    s.seek(SeekFrom::End(0))?;
    let end = s.stream_position()? as usize;
    s.seek(SeekFrom::Start((end - 4) as u64))?;
    let magic = s.read_u32::<LittleEndian>()?;

    if magic == PACKAGE_MAGIC {
        s.seek(SeekFrom::Start((end - 8) as u64))?;
        let meta_size = s.read_i32::<LittleEndian>()? as usize;

        s.seek(SeekFrom::Start((end - 12) as u64))?;
        let composite_count = s.read_i32::<LittleEndian>()? as usize;

        s.seek(SeekFrom::Start((end - 16) as u64))?;
        let offsets_offset = s.read_i32::<LittleEndian>()? as usize;

        s.seek(SeekFrom::Start((end - 20) as u64))?;
        let container_offset = s.read_i32::<LittleEndian>()? as usize;

        s.seek(SeekFrom::Start((end - 24) as u64))?;
        let name_offset = s.read_i32::<LittleEndian>()? as usize;

        s.seek(SeekFrom::Start((end - 28) as u64))?;
        let author_offset = s.read_i32::<LittleEndian>()? as usize;

        s.seek(SeekFrom::Start((end - 32) as u64))?;
        m.mod_file_version = s.read_i32::<LittleEndian>()?;

        s.seek(SeekFrom::Start((end - 36) as u64))?;
        m.region_lock = s.read_i32::<LittleEndian>()? != 0;

        let composite_end = end - meta_size - 4;

        // Read author, name, container
        s.seek(SeekFrom::Start(author_offset as u64))?;
        m.mod_author = read_string(s)?;

        s.seek(SeekFrom::Start(name_offset as u64))?;
        m.mod_name = read_string(s)?;

        s.seek(SeekFrom::Start(container_offset as u64))?;
        m.container = read_string(s)?;

        // Read offsets
        s.seek(SeekFrom::Start(offsets_offset as u64))?;
        let mut offsets = vec![0usize; composite_count];
        for offset in &mut offsets {
            *offset = s.read_i32::<LittleEndian>()? as usize;
        }

        // Initialize packages
        m.packages = vec![CompositePackage::default(); composite_count];

        // Read each composite package
        for (idx, package) in m.packages.iter_mut().enumerate() {
            s.seek(SeekFrom::Start(offsets[idx] as u64))?;
            read_composite_package(s, package)?;
        }

        // Set sizes for each package
        for idx in 1..m.packages.len() {
            m.packages[idx - 1].size = offsets[idx] - m.packages[idx - 1].offset;
        }

        if let Some(last) = m.packages.last_mut() {
            last.size = composite_end.max(end - meta_size) - last.offset;
        }
    } else {
        // Single package fallback
        let mut p = CompositePackage::default();
        s.seek(SeekFrom::Start(0))?;
        read_composite_package(s, &mut p)?;
        p.size = end;
        m.packages.push(p);
    }

    Ok(())
}


fn read_composite_package<R: Read + Seek>(s: &mut R, p: &mut CompositePackage) -> Result<()> {
    p.offset = s.stream_position()? as usize; // usize instead of i32
    s.seek(SeekFrom::Current(4))?;
    p.file_version = s.read_u16::<LittleEndian>()?;
    p.licensee_version = s.read_u16::<LittleEndian>()?;
    s.seek(SeekFrom::Start(p.offset as u64 + 12))?;

    let folder_name = read_string(s)?;
    if folder_name.starts_with("MOD:") {
        p.object_path = folder_name[4..].to_string();
    }

    Ok(())
}

pub fn read_game_config<R: Read>(s: &mut R) -> Result<GameConfigFile> {
    let count = s.read_i32::<LittleEndian>()?;
    let mut mods = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let enabled = s.read_i32::<LittleEndian>()? != 0;
        let file = read_string(s)?;
        let mod_name = read_string(s)?;
        let container = read_string(s)?;
        
        // We create a default ModFile and populate the fields we persisted
        let mut mod_file = ModFile::default();
        mod_file.mod_name = mod_name;
        mod_file.container = container;

        mods.push(ModEntry { file, enabled, mod_file });
    }
    Ok(GameConfigFile { mods })
}

pub fn write_game_config<W: Write>(cfg: &GameConfigFile, s: &mut W) -> Result<()> {
    let count = cfg.mods.len() as i32;
    s.write_i32::<LittleEndian>(count)?;
    for m in &cfg.mods {
        let enabled = if m.enabled { 1 } else { 0 };
        s.write_i32::<LittleEndian>(enabled)?;
        write_string(s, &m.file)?;
        
        // Save mod_name and container
        write_string(s, &m.mod_file.mod_name)?;
        write_string(s, &m.mod_file.container)?;
    }
    s.write_u32::<LittleEndian>(PACKAGE_MAGIC)?;
    Ok(())
}
