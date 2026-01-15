#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
use anyhow::Result;
use directories::ProjectDirs;
use eframe::App;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use sysinfo::{System, ProcessesToUpdate, RefreshKind, ProcessRefreshKind};
use eframe::egui::{CentralPanel, Layout};
use bincode::{encode_to_vec, decode_from_slice};
use bincode::config;
use eframe::icon_data::from_png_bytes;
use egui::{Context, IconData};
use egui::output::OpenUrl;
use std::sync::{Arc};

mod composite_mapper;
mod mod_model;
mod ui;
mod utils;

use composite_mapper::{CompositeEntry, CompositeMapperFile};
use mod_model::{GameConfigFile, ModEntry, ModFile, CompositePackage};
use ui::{buttons_ui, mod_list_ui, root_dir_ui};

const CONFIG_FILE: &str = "settings.bin";
const GAME_CONFIG_FILE: &str = "ModList.mods";
const COMPOSITE_MAPPER_FILE: &str = "CompositePackageMapper.dat";
const BACKUP_COMPOSITE_MAPPER_FILE: &str = "CompositePackageMapper.clean";
const COOKED_PC_DIR: &str = "CookedPC";
const MODS_STORAGE_DIR: &str = "CookedPC";

struct TmmApp {
    root_dir: PathBuf,
    client_dir: PathBuf,
    mods_dir: PathBuf,
    composite_mapper_path: PathBuf,
    backup_composite_mapper_path: PathBuf,
    game_config_path: PathBuf,
    wait_for_tera: bool,
    game_config: GameConfigFile,
    composite_map: CompositeMapperFile,
    backup_map: CompositeMapperFile,
    mod_list: Vec<ModEntry>,
    selected_mods: Vec<usize>,
    tera_running: bool,
    sys: System,
    last_tera_check: std::time::Instant,
    error_msg: Option<String>,
    status_msg: String,
    warning_msg: String,
    initialized: bool,
}

impl Default for TmmApp {
    fn default() -> Self {
        let mut app = Self {
            root_dir: PathBuf::new(),
            client_dir: PathBuf::new(),
            mods_dir: PathBuf::new(),
            composite_mapper_path: PathBuf::new(),
            backup_composite_mapper_path: PathBuf::new(),
            game_config_path: PathBuf::new(),
            wait_for_tera: false,
            game_config: GameConfigFile { mods: Vec::new() },
            composite_map: CompositeMapperFile::default(),
            backup_map: CompositeMapperFile::default(),
            mod_list: Vec::new(),
            selected_mods: Vec::new(),
            tera_running: false,
            sys: System::new_with_specifics(
                RefreshKind::new()
                    .with_processes(ProcessRefreshKind::everything()),
            ),
            last_tera_check: std::time::Instant::now(),
            error_msg: None,
            status_msg: String::new(),
            warning_msg: String::new(),
            initialized: false,
        };

        // Load basic config (settings.bin) to restore previous path
        app.load_app_config().ok();

        app
    }
}

impl TmmApp {
    fn initialize(&mut self) {
        // Setup Paths
        // If root_dir is empty, this will fail, and we handle it in update().
        if let Err(e) = self.setup_paths() {
            self.error_msg = Some(format!("Setup failed: {}", e));
            return;
        }

        // Load Backup Map
        match CompositeMapperFile::new(self.backup_composite_mapper_path.clone()) {
            Ok(backup) => {
                self.backup_map = backup;
                println!("[TMM] Backup Mapper Loaded.");
            }
            Err(e) => {
                self.error_msg = Some(format!("Failed to load backup mapper: {}", e));
                return;
            }
        }

        // Load Active Composite Map
        match CompositeMapperFile::new(self.composite_mapper_path.clone()) {
            Ok(map) => {
                self.composite_map = map;
                println!("[TMM] Active Mapper Loaded.");
            }
            Err(e) => {
                self.error_msg = Some(format!("Failed to load mapper: {}", e));
                return;
            }
        }

        // Load Mod List
        if let Err(e) = self.load_game_config() {
            self.error_msg = Some(format!("Failed to load mod list: {}", e));
            return;
        }
        self.mod_list = self.game_config.mods.clone();

        // Scan Mod Files (Logic from previous 'new')
        println!("[TMM] Scanning Mod Files...");
        let _mod_list_length = self.mod_list.len();
        for (_index, mod_entry) in self.mod_list.iter_mut().enumerate() {
            let filename = &mod_entry.file;
            let gpk_path = self.mods_dir.join(filename);
            
            if !gpk_path.exists() {
                continue;
            }

            let mut file = match File::open(&gpk_path) {
                Ok(f) => f,
                Err(_) => continue,
            };

            let is_raw = if mod_model::read_mod_file(&mut file, &mut mod_entry.mod_file).is_err() {
                true
            } else {
                mod_entry.mod_file.packages.len() == 1 && mod_entry.mod_file.packages[0].size == 0
            };

            let mod_container_name = filename.trim_end_matches(".gpk").to_string();

            if is_raw {
                let mod_name_stem = filename.trim_end_matches(".gpk").to_lowercase();
                let mut matched_packages = Vec::new();
                let mut found_match = false;

                for entry in self.composite_map.composite_map.values() {
                    let entry_name_stem = entry.filename.trim_end_matches(".gpk").to_lowercase();
                    if mod_name_stem.contains(&entry_name_stem) || entry_name_stem.contains(&mod_name_stem) {
                        matched_packages.push(composite_mapper::CompositeEntry {
                            filename: filename.clone(),
                            object_path: entry.object_path.clone(),
                            composite_name: entry.composite_name.clone(),
                            offset: 0,
                            size: 0,
                        });
                        found_match = true;
                    }
                }

                if found_match {
                    mod_entry.mod_file.packages = matched_packages
                        .into_iter()
                        .map(|e| mod_model::CompositePackage {
                            object_path: e.object_path,
                            offset: e.offset,
                            size: e.size,
                            ..Default::default()
                        })
                        .collect();
                    
                    if mod_entry.mod_file.mod_name.is_empty() {
                        mod_entry.mod_file.mod_name = filename.clone();
                    }
                    mod_entry.mod_file.container = mod_container_name;
                }
            } else {
                if mod_entry.mod_file.container.is_empty() {
                    mod_entry.mod_file.container = mod_container_name;
                }
            }
        }

        // 6. Apply Mods
        if !self.wait_for_tera {
            println!("[TMM] Applying Enabled Mods...");
            if let Err(e) = self.apply_enabled_mods() {
                self.error_msg = Some(format!("Startup apply failed: {:?}", e));
            } else {
                self.status_msg = "Mods applied on startup.".to_string();
            }
            self.commit_changes();
        } else {
            self.status_msg = "Ready. Waiting for TERA launch.".to_string();
        }
    }

    fn load_app_config(&mut self) -> Result<()> {
        if let Some(proj_dirs) = ProjectDirs::from("com", "borkycode", "tera-mod-manager") {
            let config_path = proj_dirs.config_dir().join(CONFIG_FILE);
            if config_path.exists() {
                let mut file = File::open(config_path)?;
                let mut buf = Vec::new();
                file.read_to_end(&mut buf)?;
                let cfg = config::standard();
                let ((root_dir, wait_for_tera), _bytes_read): ((PathBuf, bool), usize) = decode_from_slice(&buf, cfg)?;
                self.root_dir = root_dir;
                self.wait_for_tera = wait_for_tera;
            }
        }
        Ok(())
    }

    fn save_app_config(&self) -> Result<()> {
        if let Some(proj_dirs) = ProjectDirs::from("com", "borkycode", "tera-mod-manager") {
            let config_path = proj_dirs.config_dir().join(CONFIG_FILE);
            if let Some(parent) = config_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let cfg = config::standard();
            let data = encode_to_vec(
                &(self.root_dir.clone(), self.wait_for_tera),
                cfg,
            )?;
            let mut file = File::create(config_path)?;
            file.write_all(&data)?;
        }
        Ok(())
    }

    fn setup_paths(&mut self) -> Result<()> {
        self.warning_msg.clear();
        self.error_msg = None;
        if self.root_dir.as_os_str().is_empty() || !self.root_dir.exists() {
            return Ok(());
        }

        // Construct paths
        self.composite_mapper_path = self.root_dir.join(COOKED_PC_DIR).join(COMPOSITE_MAPPER_FILE);
        self.backup_composite_mapper_path = self.root_dir.join(MODS_STORAGE_DIR).join(BACKUP_COMPOSITE_MAPPER_FILE);
        
        // Ensure the mods directory exists
        if let Err(e) = fs::create_dir_all(&self.mods_dir) {
             eprintln!("Failed to create mods dir: {:?}", e);
        }

        // Check if the critical game file exists
        if !self.composite_mapper_path.exists() {
            self.warning_msg = "CompositePackageMapper.dat not found in the selected directory.".to_string();
        }

        // Perform backup
        if !self.backup_composite_mapper() {
            self.error_msg = Some("Backup Failed".to_string());
        }

        self.client_dir = self.root_dir.parent().unwrap_or(&PathBuf::new()).to_path_buf();
        self.mods_dir = self.root_dir.join(MODS_STORAGE_DIR);
        self.game_config_path = self.mods_dir.join(GAME_CONFIG_FILE);
        self.save_app_config()?;
        Ok(())
    }

    fn backup_composite_mapper(&self) -> bool {
        if self.backup_composite_mapper_path.exists() {
            return true;
        }

        if !self.composite_mapper_path.exists() {
            return false;
        }
        
        fs::copy(&self.composite_mapper_path, &self.backup_composite_mapper_path).is_ok()
    }

    fn restore_composite_mapper(&mut self) -> bool {
        if !self.backup_composite_mapper_path.exists() {
            self.error_msg = Some("Restore Failed - Missing Backup File, Please Turn Off All Mods And Restart TMM".to_string());
            return false;
        }
        fs::copy(&self.backup_composite_mapper_path, &self.composite_mapper_path).is_ok()
    }

    fn update_mods_list(&mut self, mod_data: Vec<ModEntry>) {
        self.game_config.mods = mod_data;
        self.save_game_config().ok();
    }

    // Helper to find indices of currently enabled mods that share object paths with the provided packages
    fn find_conflicting_indices(&self, packages: &[CompositePackage]) -> Vec<usize> {
        let mut conflicts = Vec::new();

        for (i, existing_mod) in self.mod_list.iter().enumerate() {
            if !existing_mod.enabled {
                continue; // Only check against active mods
            }

            // Check intersection of packages
            for new_pkg in packages {
                for existing_pkg in &existing_mod.mod_file.packages {
                    if new_pkg.object_path == existing_pkg.object_path {
                        conflicts.push(i);
                        break; 
                    }
                }
            }
        }
        conflicts
    }


    fn install_mod(&mut self, path: &Path, save: bool) -> bool {
        let target_path = self.mods_dir.join(path.file_name().unwrap_or_default());
        if fs::copy(path, &target_path).is_err() {
            self.error_msg = Some(format!("Failed to copy mod file: {:?}", path));
            return false;
        }

        let mut file = match File::open(&target_path) {
            Ok(f) => f,
            Err(_) => return false,
        };

        let mut mod_file = ModFile::default();
    
        let is_raw = if mod_model::read_mod_file(&mut file, &mut mod_file).is_err() {
            true // Failed to read, definitely raw
        } else {
            // Check if the read resulted in the "dummy" single package (size 0)
            // If mod_file.packages has 1 item with size 0, it's likely a raw fallback from read_mod_file
            mod_file.packages.len() == 1 && mod_file.packages[0].size == 0
        };

        let file_name = target_path.file_name().unwrap().to_string_lossy().to_string();

        // Logic for Raw GPKs (Fallback)
        if is_raw {
            println!("Detected Raw/Unpacked GPK. Attempting to resolve via filename matching...");

            // Try to find the mod name in the existing composite map.
            // This assumes the user named the mod file exactly as the file it replaces.
            let mod_name_stem = file_name.trim_end_matches(".gpk").to_lowercase();
            let mut matched_packages = Vec::new();
            let mut found_match = false;

            // Scan the composite map
            for entry in self.composite_map.composite_map.values() {
                let entry_name_stem = entry.filename.trim_end_matches(".gpk").to_lowercase();
                
                // Check for partial match (e.g. "S1_Elin" matches "S1_Elin_Mod")
                if mod_name_stem.contains(&entry_name_stem) || entry_name_stem.contains(&mod_name_stem) {
                    matched_packages.push(CompositePackage {
                        object_path: entry.object_path.clone(),
                        offset: 0, 
                        size: 0,
                        file_version: 0,
                        licensee_version: 0,
                    });
                    found_match = true;
                }
            }

            if found_match {
                mod_file.packages = matched_packages;
                // Since we don't have the real name, use the filename as the display name
                mod_file.mod_name = file_name.clone(); 
                // Use filename as container if empty
                if mod_file.container.is_empty() {
                    mod_file.container = file_name.trim_end_matches(".gpk").to_string();
                }
                println!("Fallback successful. Associated with {} game objects.", mod_file.packages.len());
            } else {
                self.error_msg = Some(format!(
                    "Could not auto-detect target for raw mod '{}'.\nPlease rename it to match the game file (e.g. S1_Elin_PC.gpk).", 
                    file_name
                ));
                return false;
            }
        } else {
            // Ensure container is populated even for TMM-packed mods if somehow empty
            if mod_file.container.is_empty() {
                mod_file.container = file_name.trim_end_matches(".gpk").to_string();
            }
        }

        let conflicts = self.find_conflicting_indices(&mod_file.packages);
        for &idx in &conflicts {
            if self.mod_list[idx].enabled {
                println!("[TMM] Conflict detected: Disabling '{}' in favor of '{}'", self.mod_list[idx].file, file_name);
        
                let existing_file = self.mod_list[idx].mod_file.clone();

                self.mod_list[idx].enabled = false;
                // Restore the map for the conflicting mod
                if let Err(e) = self.turn_off_mod(&existing_file, true) {
                     eprintln!("Failed to disable conflicting mod: {:?}", e);
                }
            }
        }

        let mod_entry = ModEntry {
            file: file_name.clone(),
            enabled: true,
            mod_file,
        };

        self.mod_list.push(mod_entry.clone());
        self.game_config.mods.push(mod_entry.clone());
        
        if !self.wait_for_tera {
            // Pass the filename
            if let Err(e) = self.turn_on_mod(&mod_entry.mod_file) {
                self.error_msg = Some(format!("Failed to apply new mod: {:?}", e));
            }
            self.composite_map.dirty = true;
            self.commit_changes();
        }
        
        if save {
            self.save_game_config().ok();
        }
        self.status_msg = format!("Installed {:?}", mod_entry.mod_file.mod_name);
        true
    }

    pub fn enable_mod_safely(&mut self, index: usize) -> Result<()> {
        if index >= self.mod_list.len() {
            return Ok(());
        }

        let target_mod = self.mod_list[index].clone();
        
        // Find conflicts with OTHER enabled mods
        let conflicts = self.find_conflicting_indices(&target_mod.mod_file.packages);

        // Disable conflicting mods first
        for &conflict_idx in &conflicts {
            if self.mod_list[conflict_idx].enabled {
                println!("[TMM] Disabling conflicting mod: {}", self.mod_list[conflict_idx].file);
                self.mod_list[conflict_idx].enabled = false;
                let m_file = self.mod_list[conflict_idx].mod_file.clone();
                if let Err(e) = self.turn_off_mod(&m_file, true) {
                    eprintln!("Error disabling conflicting mod: {:?}", e);
                }
            }
        }

        // Enable the target mod
        self.mod_list[index].enabled = true;
        if let Err(e) = self.turn_on_mod(&target_mod.mod_file) {
            return Err(e);
        }
        
        self.composite_map.dirty = true;
        self.update_mods_list(self.mod_list.clone());
        Ok(())
    }

    pub fn turn_on_mod(&mut self, mod_file: &ModFile) -> Result<()> {
        
        for pkg in &mod_file.packages {
            let mut entry = CompositeEntry::default();

            // Try to find the object
            if !self
                .composite_map
                .get_entry_by_incomplete_object_path(&pkg.object_path, &mut entry)
            {
                // LOG the error but DON'T bail. Continue to the next package.
                eprintln!("[TMM] Warning: Object '{}' not found in CompositeMap. Skipping.", pkg.object_path);
                continue;
            }

            // Apply patch if found
            if let Err(e) = self.composite_map.apply_patch(
                &entry.composite_name,
                &mod_file.container,
                pkg.offset,
                pkg.size,
            ) {
                eprintln!("[TMM] Warning: Failed to patch '{}': {:?}", pkg.object_path, e);
            }
        }

        Ok(())
    }


    pub fn turn_off_mod(&mut self, mod_file: &ModFile, silent: bool) -> Result<()> {
        for pkg in &mod_file.packages {
            let mut original = CompositeEntry::default();

            // Try to find the original entry in the backup (clean) map
            if self.backup_map.get_entry_by_incomplete_object_path(&pkg.object_path, &mut original) {
                self.composite_map.apply_patch(
                    &original.composite_name,
                    &original.filename,
                    original.offset,
                    original.size,
                )?;
            } else {
                let mut active_entry = CompositeEntry::default();
                if self.composite_map.get_entry_by_incomplete_object_path(&pkg.object_path, &mut active_entry) {
                    println!("[TMM] Removing new object entry: {}", pkg.object_path);
                    self.composite_map.remove_entry(&active_entry);
                    self.composite_map.dirty = true;
                } else if !silent {
                    // If we can't find it in the active map either, it's likely a data mismatch.
                    eprintln!("[TMM] Warning: Object '{}' not found in active map or backup.", pkg.object_path);
                }
            }
        }

        Ok(())
    }


    fn commit_changes(&mut self) {
        if self.composite_map.dirty {
            if let Err(e) = self
                .composite_map
                .save(&self.composite_mapper_path)
            {
                self.error_msg = Some(format!("Failed to save: {}", e));
            } else {
                self.composite_map.dirty = false;
            }
        }
    }

    fn save_button(&mut self){
        if let Err(e) = self.composite_map.save(&self.composite_mapper_path) {
                    self.error_msg = Some(format!("Save Failed {:?}", e));
                } else {
                    self.status_msg = "Manual Save Successful".to_string();
                }
    }

    fn load_game_config(&mut self) -> Result<()> {
        if self.game_config_path.exists() {
            let mut file = File::open(&self.game_config_path)?;
            self.game_config = mod_model::read_game_config(&mut file)?;
        } else {
            self.save_game_config()?;
        }
        Ok(())
    }

    fn save_game_config(&self) -> Result<()> {
        let mut file = File::create(&self.game_config_path)?;
        mod_model::write_game_config(&self.game_config, &mut file)?;
        Ok(())
    }

    fn check_tera(&mut self) -> bool {
        self.sys.refresh_processes(ProcessesToUpdate::All);

        self.sys.processes().values().any(|p| {
            p.name().eq_ignore_ascii_case("tera.exe")
        })
    }

    pub fn apply_enabled_mods(&mut self) -> Result<()> {
        // 1. Reset the composite map to the clean backup state
        self.composite_map.composite_map = self.backup_map.composite_map.clone();

        // 2. Collect enabled mods into a new Vector that owns the data (cloning).
        // This breaks the link to 'self', allowing us to call mutable methods on 'self' afterwards.
        let mods_to_apply: Vec<(ModFile, String)> = self
            .mod_list
            .iter()
            .filter(|entry| entry.enabled)
            .map(|entry| (entry.mod_file.clone(), entry.file.clone()))
            .collect();

        // 3. Apply the mods using the cloned data
        for (mod_file, filename) in mods_to_apply {
            if let Err(e) = self.turn_on_mod(&mod_file) {
                eprintln!("Failed to apply mod {}: {:?}", filename, e);
                self.error_msg = Some(format!("Failed to apply mod {}: {:?}", filename, e));
            }
        }
        
        if !self.composite_map.composite_map.is_empty() {
            self.composite_map.dirty = true;
        }
        
        Ok(())
    }

    fn disable_all_mods(&mut self) {
        let mut changes = Vec::new();

        for (i, m) in self.mod_list.iter_mut().enumerate() {
            if m.enabled {
                m.enabled = false;
                changes.push(i);
            }
        }

        // Nothing to do
        if changes.is_empty() {
            self.status_msg = "No mods were enabled.".to_string();
            return;
        }

        // Apply changes
        for &i in &changes {
            let mod_file = self.mod_list[i].mod_file.clone();

            if let Err(e) = self.turn_off_mod(&mod_file, false) {
                self.error_msg = Some(format!(
                    "Failed to disable {}: {:?}",
                    mod_file.mod_name, e
                ));
                return;
            }
        }

        // Mark composite dirty & commit
        self.composite_map.dirty = true;
        self.commit_changes();

        // Save mod list
        self.update_mods_list(self.mod_list.clone());
        self.restore_composite_mapper();
        // UI feedback
        self.selected_mods.clear();
        self.status_msg = "Backup Restored. All mods have been disabled.".to_string();
    }

}

impl App for TmmApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        ctx.set_pixels_per_point(1.1);
        // 1. Handle Initialization if not done and root dir is set
        if !self.initialized {
            if !self.root_dir.as_os_str().is_empty() {
                // We have a path, try to load.
                self.initialize();
                // If we got here without crashing, consider us initialized (even with errors, we displayed them)
                self.initialized = true;
            }
        }

        let now = std::time::Instant::now();
        let should_check = now.duration_since(self.last_tera_check) >= std::time::Duration::from_millis(10);

        if should_check {
            self.last_tera_check = now;
            let running = self.check_tera();

            if running && !self.tera_running {
                // TERA Launched
                println!("TERA launched — applying all enabled mods");
                self.status_msg = "TERA detected. Applying mods...".to_string();
                self.error_msg = None; // Clear previous errors
                
                if let Err(e) = self.apply_enabled_mods() {
                    self.error_msg = Some(format!("Apply failed: {:?}", e));
                    self.status_msg = "Failed to apply mods!".to_string();
                }
                
                if let Err(e) = self.composite_map.save(&self.composite_mapper_path) {
                    self.error_msg = Some(format!(
                        "Failed to save CompositePackageMapper.dat: {:?}",
                        e
                    ));
                    self.status_msg = "Failed to save mapper!".to_string();
                } else {
                    self.status_msg = format!(
                        "Applied {} mods successfully.",
                        self.mod_list.iter().filter(|m| m.enabled).count()
                    );
                    println!(
                        "Applied mods successfully — saved to {}",
                        self.composite_mapper_path.display()
                    );
                }
                self.tera_running = true;
            } else if !running && self.tera_running {
                // TERA Closed
                println!("TERA closed — restoring original composite map");
                self.status_msg = "TERA closed.".to_string();
                self.error_msg = None;

                if self.wait_for_tera == true {
                self.status_msg = "TERA closed. Restoring original files.".to_string();
                if self.backup_composite_mapper_path.exists() {
                    match CompositeMapperFile::new(self.backup_composite_mapper_path.clone()) {
                        Ok(backup) => {
                            self.composite_map = backup;
                            if let Err(e) = self.composite_map.save(&self.composite_mapper_path) {
                                self.error_msg = Some(format!(
                                    "Failed to restore CompositePackageMapper.dat: {:?}",
                                    e
                                ));
                                self.status_msg = "Failed to restore mapper!".to_string();
                            } else {
                                println!(
                                    "Restored from {}",
                                    self.backup_composite_mapper_path.display()
                                );
                            }
                        }
                        Err(e) => {
                            self.error_msg = Some(format!("Failed to load backup: {:?}", e));
                            self.status_msg = "Failed to load backup!".to_string();
                        },
                    }
                } else {
                    self.error_msg = Some(format!(
                        "Backup not found at {}",
                        self.backup_composite_mapper_path.display()
                    ));
                    self.status_msg = "Backup missing!".to_string();
                }}
                self.tera_running = false;
                self.commit_changes();

                // FIX: Refresh system process list completely to ensure next launch is detected
                // This simulates a "first load" state for the system monitor
                self.sys.refresh_all(); 
            }
        }

        CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Tera Mod Manager");

                // Use right-to-left layout to push content to the right side
                ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("GitHub").clicked() {
                        ui.ctx().output_mut(|o| {
                            o.open_url = Some(OpenUrl {
                                url: "https://github.com/BorkyCode".to_owned(),
                                new_tab: true, // true = open in a new browser tab
                            });
                        });
                    }

                    if ui.button("More Mods").clicked() {
                        ui.ctx().output_mut(|o| {
                            o.open_url = Some(OpenUrl {
                                url: "https://www.tumblr.com/search/tera%20mods".to_owned(),
                                new_tab: true, // true = open in a new browser tab
                            });
                        });
                    }
                    
                });
            });

            if let Some(err) = &self.error_msg {
                ui.label(egui::RichText::new(err).color(egui::Color32::RED));
            }

            if !self.warning_msg.is_empty() {
                ui.label(egui::RichText::new(&self.warning_msg).color(egui::Color32::ORANGE));
            }

            if !self.status_msg.is_empty() {
                ui.label(egui::RichText::new(&self.status_msg).color(egui::Color32::LIGHT_GREEN));
            }

            root_dir_ui(self, ui);
            buttons_ui(self, ui);
            egui::ScrollArea::vertical().show(ui, |ui| {
                mod_list_ui(self, ui);
            });
        });
    }
}

fn load_icon() -> IconData {
    let png_bytes = include_bytes!("../assets/AppIcon.png");
    from_png_bytes(png_bytes).expect("Failed to load icon.png")
}

fn main() -> eframe::Result<()> {
    let icon = load_icon();
    let viewport = egui::ViewportBuilder::default()
        .with_icon(Arc::new(icon));

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
        
    eframe::run_native(
        "Tera Mod Manager",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_theme(eframe::egui::Theme::Dark);
            
            Ok(Box::new(TmmApp::default()))
        }),
    )
}