use egui::{Ui};
use egui_extras::{Column, TableBuilder}; // <--- Add this import


use crate::TmmApp;

pub fn root_dir_ui(app: &mut TmmApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.label("Root Dir:");

        // Check if root_dir is empty to decide what text to show on the button
        let button_text = if app.root_dir.as_os_str().is_empty() {
            "Select S1Game Directory".to_string()
        } else {
            app.root_dir.display().to_string()
        };

        if ui.button(button_text).clicked() {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                app.root_dir = path;
                // Reset initialization so the update loop reloads everything with the new path
                app.initialized = false;
            }
        }
    });
}

pub fn mod_list_ui(app: &mut TmmApp, ui: &mut Ui) {
    let mut changes = Vec::new();

    // Define table styling
    let row_height = 30.0;
    let _text_height = egui::FontId::default().size;
    
    egui::ScrollArea::vertical().show(ui, |ui| {
        // Create the table
        TableBuilder::new(ui)
            .striped(true)
            .resizable(false)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::auto())
            .column(Column::initial(200.0).at_least(100.0))
            .column(Column::initial(150.0).at_least(60.0))
            .column(Column::remainder())
            .header(20.0, |mut header| {
                header.col(|ui| { ui.with_layout(
                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                    |ui| {
                        ui.strong("Toggle");
                    },
                );  
            });
                header.col(|ui| { ui.strong("Name"); });
                header.col(|ui| { ui.strong("Author"); });
                header.col(|ui| { ui.strong("File"); });
            })
            .body(|mut body| {
            for (i, m) in app.mod_list.iter_mut().enumerate() {

            // --- Allocate row rect & response ---
            let ui = body.ui_mut();
            let cursor = ui.cursor().min;
            let width = ui.available_width();

            let row_response = ui.allocate_rect(
                egui::Rect::from_min_size(
                    cursor,
                    egui::vec2(width, row_height),
                ),
                egui::Sense::click(),
            );

            let row_rect = row_response.rect;

            // --- Theme-aware colors ---
            let visuals = ui.visuals().clone();
            let selection_color = visuals.selection.bg_fill;
            let hover_color = visuals.widgets.hovered.bg_fill;

            // --- Paint background (BEFORE widgets) ---
            if app.selected_mods.contains(&i) {
                ui.painter().rect_filled(row_rect, 4.0, selection_color);
            } else if row_response.hovered() {
                ui.painter().rect_filled(row_rect, 4.0, hover_color);
            }

            // --- Draw row contents ---
            body.row(row_height, |mut row| {
                // Checkbox
                row.col(|ui| {
                     ui.with_layout(
                        egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                        |ui| {
                            let mut enabled = m.enabled;
                            if ui.checkbox(&mut enabled, "").changed() {
                                m.enabled = enabled;
                                changes.push((i, enabled));
                            }
                        },
                    );
                });

                row.col(|ui| { ui.label(&m.mod_file.mod_name); });
                row.col(|ui| { ui.label(&m.mod_file.mod_author); });
                row.col(|ui| { ui.label(&m.file); });
            });

            // --- Single click = selection ---
            if row_response.clicked() {
                if app.selected_mods.contains(&i) {
                    app.selected_mods.retain(|&x| x != i);
                } else {
                    app.selected_mods.push(i);
                }
            }

            // --- Double click = toggle enable ---
            if row_response.double_clicked() {
                let new_state = !m.enabled;
                m.enabled = new_state;
                changes.push((i, new_state));
            }
        }
    })
    });

    // Apply Logic based on changes (identical to previous implementation)
    if !changes.is_empty() {
        for &(i, enabled) in &changes {
            // Determine if we are enabling or disabling
            if enabled {
                // Use safe enable for conflict handling
                if let Err(e) = app.enable_mod_safely(i) {
                    app.error_msg = Some(format!("Turn on failed: {:?}", e));
                } else {
                    app.status_msg = format!("Enabled: {}", app.mod_list[i].mod_file.mod_name);
                }
            } else {
                // Disable logic (conflicts don't matter here, just turn off)
                app.mod_list[i].enabled = false;
                if !app.wait_for_tera {
                    let mod_file = app.mod_list[i].mod_file.clone();
                    if let Err(e) = app.turn_off_mod(&mod_file, false) {
                        app.error_msg = Some(format!("Turn off failed: {:?}", e));
                    } else {
                        app.status_msg = format!("Disabled: {}", app.mod_list[i].mod_file.mod_name);
                    }
                    app.composite_map.dirty = true;
                }
            }
        }

        app.update_mods_list(app.mod_list.clone());

        if !app.wait_for_tera {
            app.commit_changes();
        } else {
            let status = if changes[0].1 { "Enabled" } else { "Disabled" };
            app.status_msg = format!("{} (pending TERA launch).", status);
        }
    }
}

pub fn buttons_ui(app: &mut TmmApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        if ui.button("Add").clicked() {
            if let Some(path) = rfd::FileDialog::new().pick_file() {
                app.install_mod(&path, true);
            }
        }
        if ui.button("Remove").clicked() && !app.selected_mods.is_empty() {
            app.selected_mods.sort_unstable_by(|a, b| b.cmp(a));
            for &idx in &app.selected_mods {
                app.mod_list.remove(idx);
            }
            app.update_mods_list(app.mod_list.clone());
            app.selected_mods.clear();
            app.status_msg = "Removed selected mods.".to_string();
        }
        if ui.button("On").clicked() {
            let selected = app.selected_mods.clone();
            if selected.is_empty() {
                app.status_msg = "No mods selected.".to_string();
            }
            for idx in selected {
                // Use the new safe method that handles conflicts
                if let Err(e) = app.enable_mod_safely(idx) {
                    app.error_msg = Some(format!("Turn on failed: {:?}", e));
                } else {
                    app.status_msg = format!("Enabled: {}", app.mod_list[idx].mod_file.mod_name);
                }
            }
            // Commit changes if not waiting
            if !app.wait_for_tera {
                app.commit_changes();
            } else {
                app.status_msg = format!("{} mods enabled (pending TERA launch).", app.selected_mods.len());
            }
        }

        if ui.button("Off").clicked() {
            let selected = app.selected_mods.clone();
            if selected.is_empty() {
                app.status_msg = "No mods selected.".to_string();
            }
            for idx in selected {
                app.mod_list[idx].enabled = false;
                if !app.wait_for_tera {
                    let mod_file = app.mod_list[idx].mod_file.clone();
                    if let Err(e) = app.turn_off_mod(&mod_file, false) {
                        app.error_msg = Some(format!("Turn off failed: {:?}", e));
                    } else {
                        app.status_msg = format!("Disabled: {}", app.mod_list[idx].mod_file.mod_name);
                    }
                    app.composite_map.dirty = true;
                }
            }
            app.update_mods_list(app.mod_list.clone());

            if !app.wait_for_tera {
                app.commit_changes();
            } else {
                app.status_msg = format!("{} mods disabled (pending TERA launch).", app.selected_mods.len());
            }
        }
        // ... Restore, Apply Now, Wait for TERA buttons remain the same ...
        if ui.button("Restore").clicked() {
            app.restore_composite_mapper();
            app.disable_all_mods();
        }

        if ui.button("Apply Now").clicked() {
            app.save_button();
        }
        
        if ui.checkbox(&mut app.wait_for_tera, "Wait for TERA").changed() {
            if let Err(e) = app.save_app_config() {
                app.error_msg = Some(format!("Failed to save settings: {}", e));
            } else {
                let state = if app.wait_for_tera { "enabled" } else { "disabled" };
                app.status_msg = format!("Wait for TERA {}.", state);
            }
        }
    });
}
