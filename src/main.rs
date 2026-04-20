// Hide the Windows console window when building the release version!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use affinity;
use eframe::egui;
use ini::Ini;
use rfd::FileDialog;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::collections::HashMap;

fn main() -> Result<(), eframe::Error> {
    // --- NEW: CPU Affinity Setup ---
    let core_count = affinity::get_core_num();
    let mut worker_cores = Vec::new();

    // Define how many logical threads make up your first physical core.
    // Standard Hyper-Threading/SMT uses 2. Change this to 4 if applicable.
    let threads_per_core = 2;

    // Only apply custom affinity if there are enough cores to leave some for background workers
    if core_count > threads_per_core {
        // Assign the UI to the LAST threads to avoid contention with system processes on core 0
        let ui_cores: Vec<usize> = (core_count - threads_per_core..core_count).collect();
        if let Err(e) = affinity::set_thread_affinity(ui_cores) {
            eprintln!("Failed to set UI thread affinity: {}", e);
        }
        // Assign all OTHER cores (from 0 up to the UI cores) to the background worker threads
        worker_cores = (0..core_count - threads_per_core).collect();
    }

    let mut viewport = egui::ViewportBuilder::default().with_inner_size([840.0, 500.0]);

    // Load the icon data from the embedded .ico file
    match image::load_from_memory(include_bytes!("my_icon.ico")) {
        Ok(img) => {
            let img = img.into_rgba8();
            let (width, height) = img.dimensions();
            let icon_data = egui::IconData {
                rgba: img.into_raw(),
                width,
                height,
            };
            viewport = viewport.with_icon(std::sync::Arc::new(icon_data));
        }
        Err(e) => eprintln!("Failed to load application icon: {}", e),
    }

    // Set the default theme to dark
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default() // Use viewport for window size
    };

    // Run the eframe application
    eframe::run_native(
        "Mod-O-Matic",
        options,
        Box::new(|cc| {
            // Set the visual style (theme)
            // cc.egui_ctx.set_visuals(egui::Visuals::dark()); // We will use a custom style below

            // --- FONT SETUP ---
            let mut fonts = egui::FontDefinitions::default();

            // Install our custom font.
            // This will fail if the file doesn't exist at compile time.
            let custom_font = egui::FontData::from_static(include_bytes!("my_font.ttf"));

            fonts.font_data.insert(
                "my_font".to_owned(),
                custom_font,
            );

            // Put my font first (highest priority) for proportional text:
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "my_font".to_owned());

            // Set the fonts.
            cc.egui_ctx.set_fonts(fonts);

            // --- STYLE SETUP ---
            let mut style = egui::Style {
                visuals: egui::Visuals::dark(),
                ..Default::default()
            };

        // Make text box backgrounds and other extreme backgrounds slightly darker
        style.visuals.extreme_bg_color = egui::Color32::from_gray(25);

        // Make buttons transparent by default with a faint outline
        style.visuals.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
        style.visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 20));

        // Glowing outline on hover, still transparent fill
        style.visuals.widgets.hovered.bg_fill = egui::Color32::TRANSPARENT;
        style.visuals.widgets.hovered.bg_stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(50, 150, 255));

        // Brighten up slightly when clicked
        style.visuals.widgets.active.bg_fill = egui::Color32::from_rgba_unmultiplied(50, 150, 255, 30);
        style.visuals.widgets.active.bg_stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 200, 255));

        // Force all default text to be a crisp, bright white!
        style.visuals.widgets.noninteractive.fg_stroke.color = egui::Color32::WHITE;
        style.visuals.widgets.inactive.fg_stroke.color = egui::Color32::WHITE;
        style.visuals.widgets.hovered.fg_stroke.color = egui::Color32::WHITE;
        style.visuals.widgets.active.fg_stroke.color = egui::Color32::WHITE;

            style.text_styles = [
            (egui::TextStyle::Heading, egui::FontId::new(22.0, egui::FontFamily::Proportional)),
                (egui::TextStyle::Body, egui::FontId::new(18.0, egui::FontFamily::Proportional)),
                (egui::TextStyle::Monospace, egui::FontId::new(18.0, egui::FontFamily::Proportional)),
                (egui::TextStyle::Button, egui::FontId::new(18.0, egui::FontFamily::Proportional)),
                (egui::TextStyle::Small, egui::FontId::new(14.0, egui::FontFamily::Proportional)),
            ]
            .into();
            cc.egui_ctx.set_style(style);

            Box::new(FileReplacerApp::new(worker_cores))
        }),
    )
}

// Enum to communicate progress and result from the background thread
enum JobMessage {
    Progress(usize, usize),
    Done(ProcessingResult),
}

// Enum for messages from the background file scanning thread
enum ScanMessage {
    ScanFinished {
        new_master: Option<PathBuf>,
        new_targets: Vec<PathBuf>,
        discarded_count: usize,
    },
}

// Enum for messages from the background validation thread
enum ValidationMessage {
    ValidationComplete {
        // The list of files that passed validation.
        validated_targets: Vec<PathBuf>,
        // The master file to set after validation (might be reverted).
        new_master: Option<PathBuf>,
        // A warning message to display to the user, if any.
        warning: Option<String>,
    },
}
// Enum to communicate the result from the background thread
enum ProcessingResult {
    Success,
    Error(String),
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum ExportMode {
    SafeExport,
    Overwrite,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum SaveMode {
    NewMergedFile,
    OverwriteBackup,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum AppMode {
    ReplaceOMatic,
    SortOMatic,
    MergeOMatic,
    AuthentOMatic,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum SortMode {
    GroupByExtension,
    PreserveStructure,
    SingleFolder,
}

// Struct to hold the results of our file authenticity scan
struct AuthResult {
    path: PathBuf,
    extension: String,
    detected_format: String,
    magic_hex: String,
    magic_hex_16: String,
    is_warning: bool,
}

struct FileReplacerApp {
    replacement_file: Option<PathBuf>,
    files_to_replace: Vec<PathBuf>,
    export_mode: ExportMode,
    copy_to_location: Option<PathBuf>,
    // A channel receiver to get the result from the background thread
    job_receiver: Option<Receiver<JobMessage>>,
    job_progress: Option<(usize, usize)>,
    last_error: Option<String>,
    file_warning: Option<String>,
    disable_safety_checks: bool,
    // --- New fields for background scanning ---
    scan_receiver: Option<Receiver<ScanMessage>>,
    is_scanning: bool,
    // --- New fields for background validation ---
    validation_receiver: Option<Receiver<ValidationMessage>>,
    is_validating: bool,
    worker_cores: Vec<usize>,

    // --- New fields for Sort-O-Matic ---
    app_mode: AppMode,
    sort_extensions: String,
    files_to_sort: Vec<PathBuf>,
    sort_destination: Option<PathBuf>,
    sort_job_receiver: Option<Receiver<JobMessage>>,
    sort_job_progress: Option<(usize, usize)>,
    sort_mode: SortMode,
    sort_put_in_new_folder: bool,

    // --- New fields for Merge-O-MaticINI ---
    merge_main_file: Option<PathBuf>,
    merge_tweak_files: Vec<PathBuf>,
    merge_save_mode: SaveMode,
    merge_status_text: String,
    merge_job_receiver: Option<Receiver<JobMessage>>,
    merge_job_progress: Option<(usize, usize)>,

    // --- New fields for Authent-O-Matic ---
    auth_results: Vec<AuthResult>,
    auth_scan_receiver: Option<Receiver<Vec<AuthResult>>>,
    auth_success_msg: Option<String>,
    auth_warning_msg: Option<String>,
    auth_ignore_xml: bool,
}

impl FileReplacerApp {
    fn new(worker_cores: Vec<usize>) -> Self {
        Self {
            worker_cores,
            ..Default::default()
        }
    }
}

impl Default for FileReplacerApp {
    fn default() -> Self {
        Self {
            replacement_file: None,
            files_to_replace: Vec::new(),
            export_mode: ExportMode::SafeExport,
            copy_to_location: None,
            job_receiver: None,
            job_progress: None,
            last_error: None,
            file_warning: None,
            disable_safety_checks: false,
            // --- New fields ---
            scan_receiver: None,
            is_scanning: false,
            // --- New fields ---
            validation_receiver: None,
            is_validating: false,
            worker_cores: Vec::new(),
            // --- Sort-O-Matic ---
            app_mode: AppMode::ReplaceOMatic,
            sort_extensions: "png, jpg, dds".to_string(),
            files_to_sort: Vec::new(),
            sort_destination: None,
            sort_job_receiver: None,
            sort_job_progress: None,
            sort_mode: SortMode::GroupByExtension,
            sort_put_in_new_folder: true,
            // --- Merge-O-MaticINI ---
            merge_main_file: None,
            merge_tweak_files: Vec::new(),
            merge_save_mode: SaveMode::NewMergedFile,
            merge_status_text: "Ready for files...".to_string(),
            merge_job_receiver: None,
            merge_job_progress: None,
            // --- Authent-O-Matic ---
            auth_results: Vec::new(),
            auth_scan_receiver: None,
            auth_success_msg: None,
            auth_warning_msg: None,
            auth_ignore_xml: true,
        }
    }
}

// Implement the eframe::App trait to turn our struct into a GUI application
impl eframe::App for FileReplacerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let previous_safety = self.disable_safety_checks;
        let previous_top_file = self.replacement_file.clone();

        // --- Check for background job completion and streaming progress ---
        let mut job_done = false;
        if let Some(receiver) = &self.job_receiver {
            for msg in receiver.try_iter() {
                match msg {
                    JobMessage::Progress(curr, total) => {
                        self.job_progress = Some((curr, total));
                    }
                    JobMessage::Done(result) => {
                        match result {
                            ProcessingResult::Success => {
                                self.files_to_replace.clear();
                                self.last_error = None;
                            }
                            ProcessingResult::Error(e) => {
                                self.last_error = Some(e);
                            }
                        }
                        job_done = true;
                    }
                }
            }
        }
        if job_done {
            self.job_receiver = None; // Job is done
            self.job_progress = None;
        }

        // --- Check for background SCAN job completion ---
        if let Some(receiver) = &self.scan_receiver {
            // Use `try_recv` to avoid blocking the UI thread
            if let Ok(msg) = receiver.try_recv() {
                let ScanMessage::ScanFinished { new_master, new_targets, discarded_count } = msg;
                match self.app_mode {
                    AppMode::ReplaceOMatic => {
                        // If the scan decided on a new master file (because ours was empty), set it.
                        if self.replacement_file.is_none() {
                            self.replacement_file = new_master;
                        }
                        // Add the new target files to our list, avoiding duplicates.
                        self.files_to_replace.extend(new_targets);
                        self.files_to_replace.sort();
                        self.files_to_replace.dedup();
                        // Show a warning if files were discarded.
                        if discarded_count > 0 {
                            self.file_warning = Some(format!("{} incompatible files were automatically discarded to prevent corruption.", discarded_count));
                        }
                    }
                    AppMode::SortOMatic => {
                        self.files_to_sort.extend(new_targets);
                        self.files_to_sort.sort();
                        self.files_to_sort.dedup();
                    }
                    AppMode::MergeOMatic => {} // Merge-O-Matic handles files directly and doesn't use the background scanner
                    AppMode::AuthentOMatic => {} // Authent-O-Matic uses its own dedicated auth_scan_receiver
                }
                self.scan_receiver = None; // Job is done
                self.is_scanning = false;
            }
        }

        // --- Check for background VALIDATION job completion ---
        if let Some(receiver) = &self.validation_receiver {
            if let Ok(msg) = receiver.try_recv() {
                match msg {
                    ValidationMessage::ValidationComplete { validated_targets, new_master, warning } => {
                        self.files_to_replace = validated_targets;
                        self.replacement_file = new_master;

                        if let Some(new_warning) = warning {
                            if let Some(existing) = &self.file_warning {
                                self.file_warning = Some(format!("{}\n\n{}", existing, new_warning));
                            } else {
                                self.file_warning = Some(new_warning);
                            }
                        }
                    }
                }
                self.validation_receiver = None;
                self.is_validating = false;
            }
        }

        // --- Check for background SORT job completion ---
        let mut sort_job_done = false;
        if let Some(receiver) = &self.sort_job_receiver {
            for msg in receiver.try_iter() {
                match msg {
                    JobMessage::Progress(curr, total) => {
                        self.sort_job_progress = Some((curr, total));
                    }
                    JobMessage::Done(result) => {
                        match result {
                            ProcessingResult::Success => {
                                self.files_to_sort.clear();
                                self.last_error = None;
                            }
                            ProcessingResult::Error(e) => {
                                self.last_error = Some(e);
                            }
                        }
                        sort_job_done = true;
                    }
                }
            }
        }
        if sort_job_done {
            self.sort_job_receiver = None;
            self.sort_job_progress = None;
        }

        // --- Check for background MERGE job completion ---
        let mut merge_job_done = false;
        if let Some(receiver) = &self.merge_job_receiver {
            for msg in receiver.try_iter() {
                match msg {
                    JobMessage::Progress(curr, total) => {
                        self.merge_job_progress = Some((curr, total));
                    }
                    JobMessage::Done(result) => {
                        match result {
                            ProcessingResult::Success => {
                                self.merge_status_text = "Merge completed successfully!".to_string();
                                self.last_error = None;
                            }
                            ProcessingResult::Error(e) => {
                                self.last_error = Some(e);
                                self.merge_status_text = "Merge failed with errors.".to_string();
                            }
                        }
                        merge_job_done = true;
                    }
                }
            }
        }
        if merge_job_done {
            self.merge_job_receiver = None;
            self.merge_job_progress = None;
        }

        // --- Check for background AUTH scan completion ---
        if let Some(receiver) = &self.auth_scan_receiver {
            if let Ok(mismatches) = receiver.try_recv() {
                if mismatches.is_empty() {
                    self.auth_success_msg = Some("Scan complete!\n\nAll scanned files have perfectly matching headers and extensions. No issues found!".to_string());
                } else {
                    let xml_warnings_count = mismatches.iter().filter(|r| r.is_warning).count();
                    if xml_warnings_count > 0 && !self.auth_ignore_xml {
                        self.auth_warning_msg = Some(format!("Found {} file(s) flagged with a yellow warning (⚠️).\n\nThese are XML or plain-text documents using custom game extensions (like .meta or .config).\n\nThis is extremely common in game mods, so these are highly likely to be harmless FALSE POSITIVES.", xml_warnings_count));
                    }
                    self.auth_results.extend(mismatches);
                }
                self.auth_scan_receiver = None;
                self.is_scanning = false;
            }
        }


        // --- UI Definition ---
        // We define the panels once, and then fill them with content based on the AppMode.
        // This avoids layout loops caused by defining conflicting panels.
        let top_panel_response = egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            // --- App Tabs UI ---
            ui.add_space(10.0); // Add a nice cushion above the dropdown

            // Use selectable_value widgets to create a tab-like interface
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.app_mode, AppMode::ReplaceOMatic, "Replace-O-Matic");
                ui.selectable_value(&mut self.app_mode, AppMode::SortOMatic, "Sort-O-Matic");
                ui.selectable_value(&mut self.app_mode, AppMode::MergeOMatic, "Merge-O-MaticINI");
                ui.selectable_value(&mut self.app_mode, AppMode::AuthentOMatic, "Authent-O-Matic");
            });
            ui.separator();

            match self.app_mode {
                AppMode::ReplaceOMatic => {
            ui.add_space(15.0); // Create a clear cushion pushing down from the top edge

            ui.horizontal(|ui| {
                ui.heading("File to Replace With");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Use ui.scope so our red theme override doesn't leak!
                    ui.scope(|ui| {
                        ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 50, 50));
                        ui.visuals_mut().widgets.active.bg_fill = egui::Color32::from_rgba_unmultiplied(255, 50, 50, 30);
                        ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 100, 100));
                        ui.visuals_mut().selection.bg_fill = egui::Color32::from_rgb(200, 40, 40); // The checkmark fill color
                        let tooltip = "DANGER: Bypassing verification allows replacing files with mismatched formats.\nThis can corrupt your game data!";
                        let checkbox_text = egui::RichText::new("DANGER: Disable Format Safety Checks").strong();
                        ui.checkbox(&mut self.disable_safety_checks, checkbox_text).on_hover_text(tooltip);
                    });
                });
            });

            // Add a small space to perfectly align the text box under the heading
            ui.add_space(3.0);

            ui.horizontal(|ui| {
                // Place fixed-size buttons first to ensure a stable layout.
                let find_button = egui::Button::new("Find...");
                if ui.add(find_button).clicked() {
                    if let Some(file) = FileDialog::new().pick_file() {
                        self.replacement_file = Some(file);
                    }
                }
                let is_top_loaded = self.replacement_file.is_some();
                if ui.add_enabled(is_top_loaded, egui::Button::new("X")).on_hover_text("Clear replacement file").clicked() {
                    self.replacement_file = None;
                }

                // Then, add the TextEdit which will fill the remaining horizontal space.
                let mut path_str = path_buf_to_string(&self.replacement_file);
                ui.add(egui::TextEdit::singleline(&mut path_str).interactive(false).desired_width(f32::INFINITY));
            });
                }
                AppMode::SortOMatic => {
                    ui.add_space(15.0);
                    ui.heading("File Extensions to Sort");
                    ui.add_space(3.0);
                    ui.label("Enter ANY extensions separated by commas (e.g., png, txt, dds, xml, etc.)");
                    ui.add(egui::TextEdit::singleline(&mut self.sort_extensions).desired_width(f32::INFINITY));
                }
                AppMode::MergeOMatic => {
                    ui.add_space(15.0);
                    ui.horizontal(|ui| {
                        ui.heading("1. Main INI File");
                        ui.add_space(3.0);
                        ui.label("(The base file to apply tweaks to)");
                    });
                    ui.add_space(3.0);
                    ui.horizontal(|ui| {
                        let find_button = egui::Button::new("Find...");
                        if ui.add(find_button).clicked() {
                            if let Some(file) = FileDialog::new().add_filter("INI", &["ini"]).pick_file() {
                                self.merge_main_file = Some(file);
                            }
                        }
                        let is_top_loaded = self.merge_main_file.is_some();
                        if ui.add_enabled(is_top_loaded, egui::Button::new("X")).on_hover_text("Clear main file").clicked() {
                            self.merge_main_file = None;
                        }
                        let mut path_str = path_buf_to_string(&self.merge_main_file);
                        ui.add(egui::TextEdit::singleline(&mut path_str).interactive(false).desired_width(f32::INFINITY));
                    });
                }
                AppMode::AuthentOMatic => {
                    ui.add_space(15.0);
                    ui.heading("File Header Analyzer");
                    ui.add_space(3.0);
                    ui.label("Drop folders below to hunt down files with mismatched extensions or corrupted/empty headers.");
                }
            }
        });

        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            match self.app_mode {
                AppMode::ReplaceOMatic => {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let has_files_to_replace = !self.files_to_replace.is_empty();
                if ui.add_enabled(has_files_to_replace, egui::Button::new("Clear")).clicked() {
                    self.files_to_replace.clear();
                    self.last_error = None;
                }

                let is_top_loaded = self.replacement_file.is_some();
                if ui.add_enabled(is_top_loaded, egui::Button::new("Find...")).on_disabled_hover_text("Select a Replacement File first").clicked() {
                    if let Some(files) = FileDialog::new().pick_files() {
                        // Convert Vec<PathBuf> to Vec<egui::DroppedFile> to reuse the existing scanner
                        let dropped_files = files.into_iter().map(|path| egui::DroppedFile { path: Some(path), ..Default::default() }).collect();

                        // Trigger the background scan, same as drag-and-drop, to prevent UI freezes
                        self.is_scanning = true;
                        let (sender, receiver) = mpsc::channel();
                        self.scan_receiver = Some(receiver);

                        let replacement_file = self.replacement_file.clone();
                        let disable_safety_checks = self.disable_safety_checks;
                        let worker_cores = self.worker_cores.clone();
                        self.file_warning = None; // Clear previous warnings

                        thread::spawn(move || {
                            if !worker_cores.is_empty() {
                                if let Err(e) = affinity::set_thread_affinity(worker_cores) {
                                    eprintln!("Failed to set scanning thread affinity: {}", e);
                                }
                            }
                            let result = perform_file_scan(
                                dropped_files,
                                replacement_file,
                                disable_safety_checks,
                            );
                            sender.send(result).ok();
                        });
                    }
                }

                ui.add_space(10.0);
                ui.label(format!("Files: {}", self.files_to_replace.len()));

                // Push the Replace button perfectly to the right side!
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Disable the replace button while a job is running, or if prerequisites aren't met
                    let is_processing = self.job_receiver.is_some();
                    let can_replace = self.replacement_file.is_some()
                        && !self.files_to_replace.is_empty()
                        && (self.export_mode == ExportMode::Overwrite || self.copy_to_location.is_some());

                    ui.scope(|ui| {
                        if !is_processing && can_replace {
                            // Override the global blue theme with neon green for the "Go" state!
                            ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(50, 255, 80));
                            ui.visuals_mut().widgets.active.bg_fill = egui::Color32::from_rgba_unmultiplied(50, 255, 80, 30);
                            ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 255, 120));
                        }

                        let replace_button = egui::Button::new("Replace");

                        ui.add_enabled(!is_processing && can_replace, replace_button)
                            .on_hover_text("Replace target files with the source file")
                            .clicked().then(|| {
                                self.last_error = None;
                                // --- Start the replacement job in a background thread ---
                                let (sender, receiver) = mpsc::channel();
                            self.job_receiver = Some(receiver);
                            self.job_progress = Some((0, self.files_to_replace.len()));

                                let replacement_file = self.replacement_file.clone();
                                let files_to_replace = self.files_to_replace.clone();
                                let export_mode = self.export_mode;
                                let copy_to_location = self.copy_to_location.clone();
                                let worker_cores = self.worker_cores.clone();

                                thread::spawn(move || {
                                    // --- NEW: Set worker thread affinity ---
                                    if !worker_cores.is_empty() {
                                        if let Err(e) = affinity::set_thread_affinity(worker_cores) {
                                            // Not a fatal error, so just print it. The thread will run on any core.
                                            eprintln!("Failed to set replacement thread affinity: {}", e);
                                        }
                                    }

                                    let result = perform_replacement(
                                        replacement_file,
                                        files_to_replace,
                                        export_mode,
                                        copy_to_location,
                                    sender.clone(),
                                    );
                                sender.send(JobMessage::Done(result)).ok();
                                });
                            });
                    });
                });
            });

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label("Mode:");
                egui::ComboBox::from_id_source("export_mode_combo")
                    .selected_text(format!("{:?}", self.export_mode))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.export_mode, ExportMode::SafeExport, "Safe Export")
                            .on_hover_text("Create a new folder with copies of the replaced files (Recommended).");
                        ui.selectable_value(&mut self.export_mode, ExportMode::Overwrite, "Overwrite")
                            .on_hover_text("Directly overwrite the original files in-place (Advanced).");
                    });
            });

            ui.add_space(5.0);

            ui.horizontal(|ui| {
                if self.export_mode == ExportMode::SafeExport {
                    ui.label("Safe Export To:");
                } else {
                    ui.label("Backup Originals To (Optional):");
                }
                let select_button = egui::Button::new("Select...");
                if ui.add(select_button).clicked() {
                    if let Some(folder) = FileDialog::new().pick_folder() {
                        self.copy_to_location = Some(folder);
                    }
                }
                if self.copy_to_location.is_some() {
                    if ui.button("X").on_hover_text("Clear folder selection").clicked() {
                        self.copy_to_location = None;
                    }
                }
                let mut path_str = path_buf_to_string(&self.copy_to_location);
                ui.add(egui::TextEdit::singleline(&mut path_str).interactive(false).desired_width(f32::INFINITY));
            });

            // Display any errors that occurred
            if let Some(error) = &self.last_error {
                ui.add_space(5.0);
                ui.colored_label(egui::Color32::RED, format!("Error: {}", error));
            }
        }
        AppMode::SortOMatic => {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!self.files_to_sort.is_empty(), egui::Button::new("Clear")).clicked() {
                            self.files_to_sort.clear();
                        }
                        ui.label(format!("Files: {}", self.files_to_sort.len()));

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let is_processing = self.sort_job_receiver.is_some();
                            let can_sort = !self.files_to_sort.is_empty() && self.sort_destination.is_some();

                            ui.scope(|ui| {
                                if !is_processing && can_sort {
                                    // Override the global blue theme with neon green for the "Go" state!
                                    ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(50, 255, 80));
                                    ui.visuals_mut().widgets.active.bg_fill = egui::Color32::from_rgba_unmultiplied(50, 255, 80, 30);
                                    ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 255, 120));
                                }

                                if ui.add_enabled(!is_processing && can_sort, egui::Button::new("Sort Files")).clicked() {
                                    self.last_error = None;
                                    let (sender, receiver) = mpsc::channel();
                                    self.sort_job_receiver = Some(receiver);
                                    self.sort_job_progress = Some((0, self.files_to_sort.len()));

                                    let files_to_sort = self.files_to_sort.clone();
                                    let sort_destination = self.sort_destination.clone().unwrap();
                                    let extensions_to_sort = self.sort_extensions.clone();
                                    let worker_cores = self.worker_cores.clone();
                                    let sort_mode = self.sort_mode;
                                    let sort_put_in_new_folder = self.sort_put_in_new_folder;

                                    thread::spawn(move || {
                                        if !worker_cores.is_empty() {
                                            if let Err(e) = affinity::set_thread_affinity(worker_cores) {
                                                eprintln!("Failed to set sort thread affinity: {}", e);
                                            }
                                        }
                                        let result = perform_sort(files_to_sort, sort_destination, extensions_to_sort, sender.clone(), sort_mode, sort_put_in_new_folder);
                                        sender.send(JobMessage::Done(result)).ok();
                                    });
                                }
                            });
                        });
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label("Sort Behavior:");
                        egui::ComboBox::from_id_source("sort_mode_combo")
                            .selected_text(format!("{:?}", self.sort_mode))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.sort_mode, SortMode::GroupByExtension, "Group by Extension")
                                    .on_hover_text("Group sorted files into sub-folders by extension (e.g., /png, /jpg).");
                                ui.selectable_value(&mut self.sort_mode, SortMode::PreserveStructure, "Preserve Structure")
                                    .on_hover_text("Recreate the original folder structure at the destination.");
                                ui.selectable_value(&mut self.sort_mode, SortMode::SingleFolder, "Single Folder")
                                    .on_hover_text("Put all sorted files directly into the destination folder, with no sub-folders.");
                            });
                        ui.add_space(15.0);
                        ui.checkbox(&mut self.sort_put_in_new_folder, "Put Output In New Folder");
                    });
                    ui.add_space(5.0);
                    ui.horizontal(|ui| {
                        if ui.button("Select Destination...").clicked() {
                            if let Some(folder) = FileDialog::new().pick_folder() {
                                self.sort_destination = Some(folder);
                            }
                        }
                        let mut path_str = path_buf_to_string(&self.sort_destination);
                        ui.add(egui::TextEdit::singleline(&mut path_str).interactive(false).desired_width(f32::INFINITY));
                    });
                    if let Some(error) = &self.last_error {
                        ui.add_space(5.0);
                        ui.colored_label(egui::Color32::RED, format!("Error: {}", error));
                    }
                }
                AppMode::MergeOMatic => {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!self.merge_tweak_files.is_empty(), egui::Button::new("Clear")).clicked() {
                            self.merge_tweak_files.clear();
                            self.merge_status_text = "Tweaks cleared.".to_string();
                        }
                        if ui.add(egui::Button::new("Find Tweaks...")).clicked() {
                            if let Some(files) = FileDialog::new().add_filter("INI", &["ini"]).pick_files() {
                                self.merge_tweak_files.extend(files);
                                self.merge_status_text = format!("Ready to merge {} tweaks.", self.merge_tweak_files.len());
                            }
                        }
                        ui.label(format!("Tweaks: {}", self.merge_tweak_files.len()));

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let is_processing = self.merge_job_receiver.is_some();
                            let can_merge = self.merge_main_file.is_some() && !self.merge_tweak_files.is_empty();

                            ui.scope(|ui| {
                                if !is_processing && can_merge {
                                    ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(50, 255, 80));
                                    ui.visuals_mut().widgets.active.bg_fill = egui::Color32::from_rgba_unmultiplied(50, 255, 80, 30);
                                    ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 255, 120));
                                }
                                if ui.add_enabled(!is_processing && can_merge, egui::Button::new("Merge Files")).clicked() {
                                    self.last_error = None;
                                    let (sender, receiver) = mpsc::channel();
                                    self.merge_job_receiver = Some(receiver);
                                    self.merge_job_progress = Some((0, self.merge_tweak_files.len()));

                                    let main_file = self.merge_main_file.clone().unwrap();
                                    let tweak_files = self.merge_tweak_files.clone();
                                    let save_mode = self.merge_save_mode;
                                    let worker_cores = self.worker_cores.clone();

                                    thread::spawn(move || {
                                        if !worker_cores.is_empty() {
                                            if let Err(e) = affinity::set_thread_affinity(worker_cores) {
                                                eprintln!("Failed to set merge thread affinity: {}", e);
                                            }
                                        }
                                        let result = perform_merge(main_file, tweak_files, save_mode, sender.clone());
                                        sender.send(JobMessage::Done(result)).ok();
                                    });
                                }
                            });
                        });
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label("Save Mode:");
                        egui::ComboBox::from_id_source("merge_save_mode")
                            .selected_text(match self.merge_save_mode {
                                SaveMode::NewMergedFile => "Create New _Merged File",
                                SaveMode::OverwriteBackup => "Overwrite Original (Backup)",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.merge_save_mode, SaveMode::NewMergedFile, "Create New _Merged File");
                                ui.selectable_value(&mut self.merge_save_mode, SaveMode::OverwriteBackup, "Overwrite Original (Backup)");
                            });
                    });
                    ui.add_space(5.0);
                    ui.label(egui::RichText::new(&self.merge_status_text).color(egui::Color32::LIGHT_GRAY));

                    if let Some(error) = &self.last_error {
                        ui.add_space(5.0);
                        ui.colored_label(egui::Color32::RED, format!("Error: {}", error));
                    }
                }
                AppMode::AuthentOMatic => {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!self.auth_results.is_empty(), egui::Button::new("Clear")).clicked() {
                            self.auth_results.clear();
                        }
                        ui.label(format!("Mismatched Files Found: {}", self.auth_results.len()));
                    });
                }
            }
        });

        let central_panel_response = egui::CentralPanel::default().show(ctx, |ui| {
            match self.app_mode {
                AppMode::ReplaceOMatic => {
            ui.heading("Files to Replace");
            ui.separator();

            if self.files_to_replace.is_empty() {
                ui.label("Drop files here...");
            } else {
                let mut action = None;
                // Use show_rows to efficiently virtualize the list, drawing only visible items!
                let row_height = 26.0;
                egui::ScrollArea::vertical().show_rows(ui, row_height, self.files_to_replace.len(), |ui, row_range| {
                    for index in row_range {
                        let file = &self.files_to_replace[index];
                        ui.horizontal(|ui| {
                            if ui.button("X").on_hover_text("Remove this file").clicked() {
                                action = Some(index);
                            }
                            let lbl_resp = ui.add(egui::Label::new(file.to_string_lossy()).sense(egui::Sense::click()))
                                .on_hover_text("Right-click to copy path");
                            lbl_resp.context_menu(|ui| {
                                if ui.button("Copy Path").clicked() {
                                    ui.ctx().output_mut(|o| o.copied_text = file.to_string_lossy().to_string());
                                    ui.close_menu();
                                }
                            });
                        });
                    }
                });

                if let Some(index) = action {
                    self.files_to_replace.remove(index);
                }
            }
                }
                AppMode::SortOMatic => {
                    ui.heading("Files to Sort");
                    ui.separator();
                    if self.files_to_sort.is_empty() {
                        ui.label("Drop files or folders here...");
                    } else {
                        let mut action = None;
                        let row_height = 26.0;
                        egui::ScrollArea::vertical().show_rows(ui, row_height, self.files_to_sort.len(), |ui, row_range| {
                            for index in row_range {
                                let file = &self.files_to_sort[index];
                                ui.horizontal(|ui| {
                                    if ui.button("X").clicked() { action = Some(index); }
                                    let lbl_resp = ui.add(egui::Label::new(file.to_string_lossy()).sense(egui::Sense::click()))
                                        .on_hover_text("Right-click to copy path");
                                    lbl_resp.context_menu(|ui| {
                                        if ui.button("Copy Path").clicked() {
                                            ui.ctx().output_mut(|o| o.copied_text = file.to_string_lossy().to_string());
                                            ui.close_menu();
                                        }
                                    });
                                });
                            }
                        });
                        if let Some(index) = action {
                            self.files_to_sort.remove(index);
                        }
                    }
                }
                AppMode::MergeOMatic => {
                    ui.heading("2. Tweak INI Files (Order Matters)");
                    ui.separator();
                    if self.merge_tweak_files.is_empty() {
                        ui.label("Drop tweak files here. Tweaks at the bottom overwrite tweaks above them.");
                    } else {
                        let mut action = None;
                        let row_height = 26.0;
                        egui::ScrollArea::vertical().show_rows(ui, row_height, self.merge_tweak_files.len(), |ui, row_range| {
                            for index in row_range {
                                let file = &self.merge_tweak_files[index];
                                ui.horizontal(|ui| {
                                    if ui.button("X").clicked() { action = Some(index); }
                                    let lbl_resp = ui.add(egui::Label::new(file.to_string_lossy()).sense(egui::Sense::click()))
                                        .on_hover_text("Right-click to copy path");
                                    lbl_resp.context_menu(|ui| {
                                        if ui.button("Copy Path").clicked() {
                                            ui.ctx().output_mut(|o| o.copied_text = file.to_string_lossy().to_string());
                                            ui.close_menu();
                                        }
                                    });
                                });
                            }
                        });
                        if let Some(index) = action {
                            self.merge_tweak_files.remove(index);
                        }
                    }
                }
                AppMode::AuthentOMatic => {
                    ui.horizontal(|ui| {
                        ui.heading("Mismatched Files");
                        ui.add_space(20.0);
                        ui.checkbox(&mut self.auth_ignore_xml, "Ignore XML");
                    });
                    ui.separator();

                    let filtered_indices: Vec<usize> = self.auth_results
                        .iter()
                        .enumerate()
                        .filter(|(_, r)| !(self.auth_ignore_xml && r.is_warning))
                        .map(|(i, _)| i)
                        .collect();

                    if self.auth_results.is_empty() {
                        ui.label("Drop files or folders here to scan...");
                    } else if filtered_indices.is_empty() {
                        ui.label("No mismatched files to display (all filtered).");
                    } else {
                        let mut action = None;
                        let row_height = 60.0; // Increased to make room for the path
                        egui::ScrollArea::vertical().show_rows(ui, row_height, filtered_indices.len(), |ui, row_range| {
                            for i in row_range {
                                let index = filtered_indices[i];
                                let result = &self.auth_results[index];
                                ui.horizontal(|ui| {
                                    if ui.button("X").clicked() { action = Some(index); }
                                    if ui.button("📂").on_hover_text("Open Containing Folder").clicked() {
                                        if let Some(parent) = result.path.parent() {
                                            std::process::Command::new("explorer").arg(parent).spawn().ok();
                                        }
                                    }
                                    let icon_color = if result.is_warning { egui::Color32::YELLOW } else { egui::Color32::RED };
                                    ui.label(egui::RichText::new("⚠️").color(icon_color).size(18.0));

                                    let vert_resp = ui.vertical(|ui| {
                                        ui.label(egui::RichText::new(result.path.file_name().unwrap_or_default().to_string_lossy()).strong().color(egui::Color32::WHITE));
                                        ui.horizontal_wrapped(|ui| {
                                            ui.spacing_mut().item_spacing.x = 4.0;
                                            ui.label(egui::RichText::new("Path:").color(egui::Color32::GRAY));
                                            ui.label(egui::RichText::new(result.path.display().to_string()).color(egui::Color32::LIGHT_GRAY));
                                        });
                                        ui.horizontal_wrapped(|ui| {
                                            ui.spacing_mut().item_spacing.x = 4.0;
                                            ui.label(egui::RichText::new("Ext:").color(egui::Color32::GRAY));
                                            ui.label(egui::RichText::new(format!(".{}", result.extension)).color(egui::Color32::LIGHT_BLUE));

                                            ui.label(egui::RichText::new("|").color(egui::Color32::GRAY));
                                            ui.label(egui::RichText::new("Detected:").color(egui::Color32::GRAY));
                                            ui.label(egui::RichText::new(&result.detected_format).color(egui::Color32::GOLD));

                                            ui.label(egui::RichText::new("|").color(egui::Color32::GRAY));
                                            ui.label(egui::RichText::new("Hex (4):").color(egui::Color32::GRAY));
                                            ui.label(egui::RichText::new(&result.magic_hex).color(egui::Color32::LIGHT_GREEN));

                                            ui.label(egui::RichText::new("|").color(egui::Color32::GRAY));
                                            ui.label(egui::RichText::new("Hex (16):").color(egui::Color32::GRAY));
                                            ui.label(egui::RichText::new(&result.magic_hex_16).color(egui::Color32::LIGHT_GREEN));
                                        });
                                    }).response;

                                    let interact_resp = ui.interact(vert_resp.rect, ui.id().with(index).with("auth"), egui::Sense::click())
                                        .on_hover_text("Right-click to copy info");

                                    interact_resp.context_menu(|ui| {
                                        if ui.button("Copy Path").clicked() {
                                            ui.ctx().output_mut(|o| o.copied_text = result.path.display().to_string());
                                            ui.close_menu();
                                        }
                                        if ui.button("Copy Hex (16)").clicked() {
                                            ui.ctx().output_mut(|o| o.copied_text = result.magic_hex_16.clone());
                                            ui.close_menu();
                                        }
                                        if ui.button("Copy Full Info").clicked() {
                                            let info = format!("File: {}\nPath: {}\nExt: .{}\nDetected: {}\nHex (4): {}\nHex (16): {}",
                                                result.path.file_name().unwrap_or_default().to_string_lossy(), result.path.display(), result.extension, result.detected_format, result.magic_hex, result.magic_hex_16);
                                            ui.ctx().output_mut(|o| o.copied_text = info);
                                            ui.close_menu();
                                        }
                                    });
                                });
                            }
                        });
                        if let Some(index) = action {
                            self.auth_results.remove(index);
                        }
                    }
                }
            }
        });

        // --- Handle Drag and Drop ---
        // After all panels are drawn, we can check for drops and see where they landed.

        // 1. Draw visual feedback based on the application state!
        // This brings back the top/bottom blue boxes without relying on buggy Windows mouse coordinates.
        if !ctx.input(|i| i.raw.hovered_files.is_empty()) {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("drop_feedback")));

            let inactive_color = egui::Color32::from_rgba_unmultiplied(50, 150, 255, 10);
            let active_color = egui::Color32::from_rgba_unmultiplied(50, 150, 255, 60);
            let stroke_color = egui::Color32::from_rgba_unmultiplied(50, 150, 255, 150);

            match self.app_mode {
                AppMode::ReplaceOMatic => {
                    let mut top_color = inactive_color;
                    let mut bottom_color = inactive_color;

                    if self.replacement_file.is_none() {
                        top_color = active_color;
                    } else {
                        bottom_color = active_color;
                    }

                    painter.rect(top_panel_response.response.rect, 4.0, top_color, egui::Stroke::new(2.0, stroke_color));
                    painter.rect(central_panel_response.response.rect, 4.0, bottom_color, egui::Stroke::new(2.0, stroke_color));
                }
                AppMode::SortOMatic => {
                    // In Sort-O-Matic, the central panel is always the drop target.
                    painter.rect(central_panel_response.response.rect, 4.0, active_color, egui::Stroke::new(2.0, stroke_color));
                }
                AppMode::MergeOMatic => {
                    let mut top_color = inactive_color;
                    let mut bottom_color = inactive_color;

                    if self.merge_main_file.is_none() {
                        top_color = active_color;
                    } else {
                        bottom_color = active_color;
                    }

                    painter.rect(top_panel_response.response.rect, 4.0, top_color, egui::Stroke::new(2.0, stroke_color));
                    painter.rect(central_panel_response.response.rect, 4.0, bottom_color, egui::Stroke::new(2.0, stroke_color));
                }
                AppMode::AuthentOMatic => {
                    // In Authent-O-Matic, the central panel is always the drop target.
                    painter.rect(central_panel_response.response.rect, 4.0, active_color, egui::Stroke::new(2.0, stroke_color));
                }
            }
        }

        // 2. Handle the actual drop
        if !ctx.input(|i| i.raw.dropped_files.is_empty()) && !self.is_scanning && !self.is_validating && self.job_receiver.is_none() && self.sort_job_receiver.is_none() {
            match self.app_mode {
                AppMode::ReplaceOMatic => {
                    let mut dropped_files = ctx.input(|i| i.raw.dropped_files.clone());
                    let pointer_pos = ctx.input(|i| i.pointer.hover_pos());

                    let drop_on_top = pointer_pos.map_or(false, |p| top_panel_response.response.rect.contains(p));
                    let drop_on_bottom = pointer_pos.map_or(false, |p| central_panel_response.response.rect.contains(p));

                    // Determine the user's intent based on drop location
                    let mut send_to_top = drop_on_top;
                    if !drop_on_top && !drop_on_bottom {
                        // If dropped outside of a specific panel, use the default convenience behavior
                        if self.replacement_file.is_none() {
                            send_to_top = true;
                        }
                    }

                    if send_to_top && !dropped_files.is_empty() {
                        // The first file from the drop goes to the top panel.
                        if let Some(path) = dropped_files.remove(0).path {
                            self.replacement_file = Some(path);
                        }
                    }

                    // Any remaining files (or all files if not dropping on top) go to the bottom panel.
                    if !dropped_files.is_empty() {
                        self.is_scanning = true;
                        let (sender, receiver) = mpsc::channel();
                        self.scan_receiver = Some(receiver);

                        let replacement_file = self.replacement_file.clone();
                        let disable_safety_checks = self.disable_safety_checks;
                        let worker_cores = self.worker_cores.clone();
                        self.file_warning = None;

                        thread::spawn(move || {
                            if !worker_cores.is_empty() {
                                if let Err(e) = affinity::set_thread_affinity(worker_cores) {
                                    eprintln!("Failed to set scanning thread affinity: {}", e);
                                }
                            }
                            let result = perform_file_scan(
                                dropped_files,
                                replacement_file,
                                disable_safety_checks,
                            );
                            sender.send(result).ok();
                        });
                    }
                }
                AppMode::SortOMatic => {
                    self.is_scanning = true;
                    let (sender, receiver) = mpsc::channel();
                    self.scan_receiver = Some(receiver);

                    let dropped_files = ctx.input(|i| i.raw.dropped_files.clone());
                    let worker_cores = self.worker_cores.clone();

                    thread::spawn(move || {
                        if !worker_cores.is_empty() {
                            if let Err(e) = affinity::set_thread_affinity(worker_cores) {
                                eprintln!("Failed to set sort-scanning thread affinity: {}", e);
                            }
                        }
                        // For sorting, we just collect all files without any validation.
                        let mut paths_to_process = Vec::new();
                        for file in dropped_files {
                            if let Some(path) = file.path {
                                if path.is_file() {
                                    paths_to_process.push(path);
                                } else if path.is_dir() {
                                    collect_files_recursively(&path, &mut paths_to_process);
                                }
                            }
                        }
                        // We can reuse the ScanMessage, just leaving the master file as None.
                        let result = ScanMessage::ScanFinished {
                            new_master: None,
                            new_targets: paths_to_process,
                            discarded_count: 0,
                        };
                        sender.send(result).ok();
                    });
                }
                AppMode::MergeOMatic => {
                    let mut dropped_files = ctx.input(|i| i.raw.dropped_files.clone());
                    let pointer_pos = ctx.input(|i| i.pointer.hover_pos());
                    let drop_on_top = pointer_pos.map_or(false, |p| top_panel_response.response.rect.contains(p));
                    let drop_on_bottom = pointer_pos.map_or(false, |p| central_panel_response.response.rect.contains(p));

                    let mut send_to_top = drop_on_top;
                    if !drop_on_top && !drop_on_bottom {
                        if self.merge_main_file.is_none() { send_to_top = true; }
                    }

                    if send_to_top && !dropped_files.is_empty() {
                        if let Some(path) = dropped_files.remove(0).path {
                            self.merge_main_file = Some(path);
                        }
                    }
                    for file in dropped_files {
                        if let Some(path) = file.path {
                            self.merge_tweak_files.push(path);
                        }
                    }
                    self.merge_status_text = format!("Ready to merge {} tweaks.", self.merge_tweak_files.len());
                }
                AppMode::AuthentOMatic => {
                    self.is_scanning = true;
                    let (sender, receiver) = mpsc::channel();
                    self.auth_scan_receiver = Some(receiver);

                    let dropped_files = ctx.input(|i| i.raw.dropped_files.clone());
                    let worker_cores = self.worker_cores.clone();

                    thread::spawn(move || {
                        if !worker_cores.is_empty() {
                            if let Err(e) = affinity::set_thread_affinity(worker_cores) {
                                eprintln!("Failed to set auth-scanning thread affinity: {}", e);
                            }
                        }
                        let mut paths_to_process = Vec::new();
                        for file in dropped_files {
                            if let Some(path) = file.path {
                                if path.is_file() { paths_to_process.push(path); }
                                else if path.is_dir() { collect_files_recursively(&path, &mut paths_to_process); }
                            }
                        }
                        let mut mismatches = Vec::new();
                        for path in paths_to_process {
                            if let Some(auth_res) = analyze_file_authenticity(&path) {
                                mismatches.push(auth_res);
                            }
                        }
                        sender.send(mismatches).ok();
                    });
                }
            }
        }

        // --- Post-Action Safety Checks (Only for Replace-O-Matic) ---
        if self.app_mode == AppMode::ReplaceOMatic {
            let safety_just_enabled = previous_safety && !self.disable_safety_checks;
            let top_file_changed = previous_top_file != self.replacement_file;

            // If a significant state change happened that requires re-validating the list, do it in the background.
            if !self.disable_safety_checks && (top_file_changed || safety_just_enabled) && !self.is_validating && self.scan_receiver.is_none() && self.job_receiver.is_none() {
                self.is_validating = true;
                let (sender, receiver) = mpsc::channel();
                self.validation_receiver = Some(receiver);

                // Clone all the necessary data to move into the thread
                let files_to_replace = self.files_to_replace.clone();
                let current_master = self.replacement_file.clone();
                let previous_master = previous_top_file; // This is the one to maybe revert to
                let worker_cores = self.worker_cores.clone();

                thread::spawn(move || {
                    if !worker_cores.is_empty() {
                        if let Err(e) = affinity::set_thread_affinity(worker_cores.clone()) {
                            eprintln!("Failed to set validation thread affinity: {}", e);
                        }
                    }
                    let result = perform_validation(
                        files_to_replace,
                        current_master,
                        previous_master,
                        top_file_changed,
                        safety_just_enabled,
                    );
                    sender.send(result).ok();
                });
            }
        }


        // Overlays need to be drawn last
        // If a REPLACEMENT job is running, show a modal overlay
        if self.job_receiver.is_some() {
            // Keep rendering at 60 FPS so the progress bar animates smoothly!
            ctx.request_repaint();

            egui::Area::new("processing_overlay".into()) // .into() converts &str to egui::Id
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    let frame = egui::Frame::window(&ctx.style()).inner_margin(20.0);
                    frame.show(ui, |ui| {
                        ui.heading("Processing...");
                        ui.add_space(10.0);

                        if let Some((curr, total)) = self.job_progress {
                            let progress = if total > 0 { curr as f32 / total as f32 } else { 0.0 };
                            ui.add(egui::ProgressBar::new(progress).show_percentage().animate(true));
                            ui.add_space(5.0);
                            ui.label(format!("Replaced {} of {} files", curr, total));
                        } else {
                            ui.spinner();
                        }
                    });
                });
        }

        // If a SORT job is running, show a modal overlay
        if self.sort_job_receiver.is_some() {
            ctx.request_repaint();
            egui::Area::new("sort_processing_overlay".into())
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    let frame = egui::Frame::window(&ctx.style()).inner_margin(20.0);
                    frame.show(ui, |ui| {
                        ui.heading("Sorting Files...");
                        ui.add_space(10.0);
                        if let Some((curr, total)) = self.sort_job_progress {
                            let progress = if total > 0 { curr as f32 / total as f32 } else { 0.0 };
                            ui.add(egui::ProgressBar::new(progress).show_percentage().animate(true));
                            ui.add_space(5.0);
                            ui.label(format!("Sorted {} of {} files", curr, total));
                        } else {
                            ui.spinner();
                        }
                    });
                });
        }

        // If a MERGE job is running, show a modal overlay
        if self.merge_job_receiver.is_some() {
            ctx.request_repaint();
            egui::Area::new("merge_processing_overlay".into())
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    let frame = egui::Frame::window(&ctx.style()).inner_margin(20.0);
                    frame.show(ui, |ui| {
                        ui.heading("Merging INI Files...");
                        ui.add_space(10.0);
                        if let Some((curr, total)) = self.merge_job_progress {
                            let progress = if total > 0 { curr as f32 / total as f32 } else { 0.0 };
                            ui.add(egui::ProgressBar::new(progress).show_percentage().animate(true));
                            ui.add_space(5.0);
                            ui.label(format!("Merged {} of {} tweaks", curr, total));
                        } else {
                            ui.spinner();
                        }
                    });
                });
        }

        // If a scan is running (used by both apps), show a modal overlay
        if self.is_scanning {
            // Keep rendering at 60 FPS so the spinner animates smoothly!
            ctx.request_repaint();

            egui::Area::new("scanning_overlay".into())
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    let frame = egui::Frame::window(&ctx.style()).inner_margin(20.0);
                    frame.show(ui, |ui| {
                        ui.heading("Scanning Files...");
                        ui.add_space(10.0);
                        ui.spinner();
                        ui.add_space(5.0);
                        ui.label("This may take a moment for large folders...");
                    });
                });
        }

        // If a validation is running, show a modal overlay
        if self.is_validating {
            // Keep rendering at 60 FPS so the spinner animates smoothly!
            ctx.request_repaint();

            egui::Area::new("validating_overlay".into())
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    let frame = egui::Frame::window(&ctx.style()).inner_margin(20.0);
                    frame.show(ui, |ui| {
                        ui.heading("Validating File List...");
                        ui.add_space(10.0);
                        ui.spinner();
                        ui.add_space(5.0);
                    });
                });
        }

        // Safety warning modal overlay
        if let Some(warning) = self.file_warning.clone() {
            egui::Area::new("warning_overlay".into())
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    let frame = egui::Frame::window(&ctx.style())
                        .inner_margin(20.0)
                        .stroke(egui::Stroke::new(2.0, egui::Color32::RED));

                    frame.show(ui, |ui| {
                        ui.heading("Safety Warning");
                        ui.add_space(10.0);
                        ui.label(&warning);
                        ui.add_space(15.0);
                        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
                            if ui.button("OK (Auto Discard)").clicked() {
                                self.file_warning = None;
                            }
                        });
                    });
                });
        }

        // Authent-O-Matic success modal overlay
        if let Some(success_msg) = self.auth_success_msg.clone() {
            egui::Area::new("auth_success_overlay".into())
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    let frame = egui::Frame::window(&ctx.style())
                        .inner_margin(20.0)
                        .stroke(egui::Stroke::new(2.0, egui::Color32::from_rgb(50, 255, 80))); // Glowing Green Border

                    frame.show(ui, |ui| {
                        ui.heading(egui::RichText::new("Everything is Perfect!").color(egui::Color32::from_rgb(50, 255, 80)));
                        ui.add_space(10.0);
                        ui.label(&success_msg);
                        ui.add_space(15.0);
                        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
                            if ui.button("OK").clicked() {
                                self.auth_success_msg = None;
                            }
                        });
                    });
                });
        }

        // Authent-O-Matic warning modal overlay (For XML False Positives)
        if let Some(warning_msg) = self.auth_warning_msg.clone() {
            egui::Area::new("auth_warning_overlay".into())
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    let frame = egui::Frame::window(&ctx.style())
                        .inner_margin(20.0)
                        .stroke(egui::Stroke::new(2.0, egui::Color32::YELLOW)); // Glowing Yellow Border

                    frame.show(ui, |ui| {
                        ui.heading(egui::RichText::new("Notice: Possible False Positives").color(egui::Color32::YELLOW));
                        ui.add_space(10.0);
                        ui.label(&warning_msg);
                        ui.add_space(15.0);
                        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
                            if ui.button("Got it!").clicked() {
                                self.auth_warning_msg = None;
                            }
                        });
                    });
                });
        }
    }
}

/// The actual INI merging logic
fn perform_merge(
    main_path: PathBuf,
    tweak_files: Vec<PathBuf>,
    save_mode: SaveMode,
    sender: mpsc::Sender<JobMessage>,
) -> ProcessingResult {
    let mut main_ini = match Ini::load_from_file(&main_path) {
        Ok(ini) => ini,
        Err(e) => return ProcessingResult::Error(format!("Error loading main file: {}", e)),
    };

    let total = tweak_files.len();
    for (index, tweak_path) in tweak_files.iter().enumerate() {
        match Ini::load_from_file(tweak_path) {
            Ok(tweak_ini) => {
                for (sec, prop) in tweak_ini.iter() {
                    let sec_name = sec.clone();
                    let mut setter = main_ini.with_section(sec_name);
                    for (k, v) in prop.iter() {
                        setter.set(k, v);
                    }
                }
            }
            Err(e) => {
                return ProcessingResult::Error(format!("Error loading {}: {}", tweak_path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(), e));
            }
        }
        sender.send(JobMessage::Progress(index + 1, total)).ok();
    }

    let out_path = match save_mode {
        SaveMode::NewMergedFile => {
            let mut path = main_path.clone();
            let file_stem = main_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "file".to_string());
            path.set_file_name(format!("{}_Merged.ini", file_stem));
            path
        }
        SaveMode::OverwriteBackup => {
            let mut backup_path = main_path.clone();
            backup_path.set_extension("bak");
            if let Err(e) = fs::copy(&main_path, &backup_path) {
                return ProcessingResult::Error(format!("Failed to create backup: {}", e));
            }
            main_path.clone()
        }
    };

    match main_ini.write_to_file(&out_path) {
        Ok(_) => ProcessingResult::Success,
        Err(e) => ProcessingResult::Error(format!("Error saving merged file: {}", e)),
    }
}

/// The actual file sorting logic
fn perform_sort(
    files_to_sort: Vec<PathBuf>,
    destination_root: PathBuf,
    extensions_str: String,
    sender: mpsc::Sender<JobMessage>,
    sort_mode: SortMode,
    sort_put_in_new_folder: bool,
) -> ProcessingResult {
    use std::collections::HashSet;
    let extensions: HashSet<String> = extensions_str.split(',').map(|s| s.trim().to_lowercase()).collect();

    // Determine the final root directory for all sorting operations.
    let final_destination_root = if sort_put_in_new_folder {
        let new_folder_name = "Sorted_Files";
        let new_root = destination_root.join(new_folder_name);
        if let Err(e) = fs::create_dir_all(&new_root) {
            return ProcessingResult::Error(format!("Failed to create directory {}: {}", new_root.display(), e));
        }
        new_root
    } else {
        destination_root
    };

    // Find the common base path ONLY if we need to preserve structure.
    let base_path_for_structure = if sort_mode == SortMode::PreserveStructure && !files_to_sort.is_empty() {
        find_common_base_path(&files_to_sort)
    } else {
        None
    };

    let total = files_to_sort.len();
    for (index, file_path) in files_to_sort.into_iter().enumerate() {
        if let Some(ext) = file_path.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase()) {
            if extensions.contains(&ext) {
                // This file should be sorted. Now figure out where it goes.
                let final_dest = match sort_mode {
                    SortMode::GroupByExtension => {
                        let dest_folder = final_destination_root.join(&ext);
                        if let Err(e) = fs::create_dir_all(&dest_folder) {
                            return ProcessingResult::Error(format!("Failed to create directory {}: {}", dest_folder.display(), e));
                        }
                        file_path.file_name().map(|name| dest_folder.join(name))
                    }
                    SortMode::SingleFolder => {
                        // Put all files directly into the destination root.
                        file_path.file_name().map(|name| final_destination_root.join(name))
                    }
                    SortMode::PreserveStructure => {
                        if let Some(ref base_path) = base_path_for_structure {
                            if let Ok(relative_path) = file_path.strip_prefix(base_path) {
                                let new_dest = final_destination_root.join(relative_path);
                                if let Some(parent) = new_dest.parent() {
                                    if let Err(e) = fs::create_dir_all(parent) {
                                        return ProcessingResult::Error(format!("Failed to create directory {}: {}", parent.display(), e));
                                    }
                                }
                                Some(new_dest)
                            } else {
                                // Fallback to flatten if stripping prefix fails for some reason
                                let dest_folder = final_destination_root.join(&ext);
                                if let Err(e) = fs::create_dir_all(&dest_folder) { return ProcessingResult::Error(format!("Failed to create directory {}: {}", dest_folder.display(), e)); }
                                file_path.file_name().map(|name| dest_folder.join(name))
                            }
                        } else {
                            // No common base path found, so just flatten.
                            let dest_folder = final_destination_root.join(&ext);
                            if let Err(e) = fs::create_dir_all(&dest_folder) { return ProcessingResult::Error(format!("Failed to create directory {}: {}", dest_folder.display(), e)); }
                            file_path.file_name().map(|name| dest_folder.join(name))
                        }
                    }
                };

                if let Some(dest) = final_dest {
                    if let Err(e) = fs::copy(&file_path, &dest) {
                        return ProcessingResult::Error(format!("Failed to copy {} to {}: {}", file_path.display(), dest.display(), e));
                    }
                }
            }
        }
        // Send progress even for files that are skipped to keep the bar moving.
        sender.send(JobMessage::Progress(index + 1, total)).ok();
    }
    ProcessingResult::Success
}

#[derive(serde::Deserialize, Clone)]
struct CustomMagic {
    extension: String,
    description: String,
}

/// Attempts to load an external `magic_numbers.json` database if it exists next to the exe
fn load_external_db() -> HashMap<Vec<u8>, CustomMagic> {
    let mut db = HashMap::new();
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let db_path = exe_dir.join("magic_numbers.json");
            if let Ok(contents) = fs::read_to_string(db_path) {
                // Expecting JSON format: { "44445320": { "extension": "dds", "description": "DDS Texture" } }
                if let Ok(parsed) = serde_json::from_str::<HashMap<String, CustomMagic>>(&contents) {
                    for (hex_str, magic_info) in parsed {
                        if let Ok(bytes) = hex::decode(hex_str.replace(" ", "")) {
                            db.insert(bytes, magic_info);
                        }
                    }
                }
            }
        }
    }
    db
}

/// Analyzes a file's magic number using the massive 'infer' library database
fn analyze_file_authenticity(path: &PathBuf) -> Option<AuthResult> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();

    // Load the runtime DB once (in a real app, you'd cache this so it doesn't read the file 50,000 times)
    // For simplicity in this implementation, we read it here.
    let external_db = load_external_db();

    let mut external_match = None;

    // 0. Check our external downloadable database first!
    if let Some(header_16) = read_file_header_16(path) {
        // Try to match up to 16 bytes against the external DB
        for (magic_bytes, info) in &external_db {
            if header_16.starts_with(magic_bytes) {
                external_match = Some((info.extension.clone(), info.description.clone()));
                break;
            }
        }
    }

    let (detected_ext, detected_name) = if let Some(custom) = external_match {
        custom // The downloaded DB takes highest priority!
    } else if let Ok(Some(kind)) = infer::get_from_path(path) {
        // 1. Check with our massively powerful 'infer' database
        (kind.extension().to_lowercase(), kind.mime_type().to_string())
    } else {
        // 2. Double filter: Check with 'file-format' database for even more coverage!
        let mut file_format_result = None;
        if let Ok(format) = file_format::FileFormat::from_file(path) { // We now process octet-stream to flag unknown files
            file_format_result = Some((format.extension().to_lowercase(), format.media_type().to_string()));
        }

        if let Some(res) = file_format_result {
            res
        } else {
            // 3. Fallback to specific game modding formats 'infer' and 'file-format' don't know natively
            // Layer 1: The Peek (Fast Magic Number)
            if let Some(magic) = read_magic_number(path) {
                let mut layer1_res = match magic {
                    [0x44, 0x44, 0x53, 0x20] => Some(("dds".to_string(), "image/dds (DirectDraw Surface)".to_string())),
                    [0x52, 0x49, 0x46, 0x46] => Some(("wav".to_string(), "audio/vnd.wave (RIFF Container)".to_string())),
                    [0x4D, 0x5A, _, _] => Some(("exe".to_string(), "application/x-msdownload (Windows Executable)".to_string())),
                    [0x42, 0x53, 0x41, 0x00] => Some(("bsa".to_string(), "application/x-bsa (Bethesda Archive)".to_string())),
                    [0x42, 0x54, 0x44, 0x58] => Some(("ba2".to_string(), "application/x-ba2 (Bethesda Archive v2)".to_string())),
                    [0x54, 0x45, 0x53, 0x34] => Some(("esp".to_string(), "application/x-tes4 (Bethesda Plugin)".to_string())),
                    [0x34, 0x12, 0xAA, 0x55] => Some(("vpk".to_string(), "application/x-vpk (Source Engine Archive)".to_string())),
                    [0x56, 0x54, 0x46, 0x00] => Some(("vtf".to_string(), "image/vtf (Valve Texture Format)".to_string())),
                    [0xE1, 0x12, 0x6F, 0x5A] => Some(("pak".to_string(), "application/x-pak (Unreal Engine Archive)".to_string())),
                    _ => None,
                };

                // If it's an EXE, let's also execute Layer 3 to see if it's actually a Self-Extracting Archive!
                if let Some((ref ext, _)) = layer1_res {
                    if ext == "exe" {
                        if let Some((_, desc)) = layer3_deep_dive(path) {
                            layer1_res = Some(("exe".to_string(), format!("application/x-msdownload (Windows Executable) + {}", desc)));
                        }
                    }
                }

                if let Some(res) = layer1_res {
                    res
                } else {
                    // Layer 2: Signature Search (1-2 KB)
                    if let Some(res) = layer2_signature_search(path) {
                        res
                    } else if let Some(res) = layer3_deep_dive(path) {
                        // Layer 3: Deep Dive (End of file trailers)
                        res
                    } else {
                        // If all layers fail, we classify it as a generic binary so we can flag it as a potential false positive.
                        ("bin".to_string(), "application/octet-stream".to_string())
                    }
                }
            } else {
                return None; // Unreadable/Empty
            }
        }
    };

    let mut is_mismatch = ext != detected_ext;
    let mut is_warning = false; // By default, a mismatch is a hard RED error.

    // 1. Check for valid aliases to resolve the mismatch. If it's an alias, it's not a mismatch.
    if is_mismatch {
        let valid_aliases = match detected_ext.as_str() {
            "jpg" | "jpeg" => vec!["jpg", "jpeg"],
            "tif" | "tiff" => vec!["tif", "tiff"],
            "heic" | "heif" => vec!["heic", "heif"],
            "mp4" => vec!["mp4", "m4v", "m4a", "m4b"],
            "zip" => vec!["jar", "apk", "osz", "ba2", "pak", "zip", "pptx", "docx", "xlsx"], // Games and MS Office use ZIP
            "mkv" => vec!["webm", "mkv"],
            "wav" => vec!["xwm", "avi", "webp", "wav"], // RIFF container aliases
            "ogg" => vec!["ogv", "oga", "ogg"],
            "exe" => vec!["exe", "dll", "sys", "ocx"], // Windows PE executables
            "esp" => vec!["esp", "esm", "esl"], // Bethesda Plugin aliases
            "doc" | "xls" | "ppt" | "ole" | "msi" | "cfb" => vec!["doc", "dot", "xls", "xlt", "ppt", "pps", "pot", "msi", "msg", "ole", "cfb"], // OLE storage containers
            "xml" => vec!["xml", "svg", "html", "htm"], // Standard XML-based formats are aliases of XML
            _ => vec![],
        };
        if valid_aliases.contains(&ext.as_str()) {
            is_mismatch = false;
        }
    }

    // 2. If it's still a mismatch, determine if it's a hard error (red) or a warning (yellow).
    if is_mismatch {
        // A mismatch is a yellow warning if the detected type is a generic container, plain text, or unknown binary.
        // These are often used for custom game files and are likely false positives (e.g. a .fntdata file detected as 'application/octet-stream').
        let generic_exts = vec!["xml", "bin", "txt", ""]; // Include empty string for files with no known extension

        let is_generic_ext = generic_exts.contains(&detected_ext.as_str());
        let is_generic_mime = detected_name.contains("octet-stream") || detected_name.starts_with("text/");

        if is_generic_ext || is_generic_mime {
            is_warning = true;
        }
    }

    if is_mismatch {
        // Grab the first 4 bytes to show in the UI for the user's reference
        let magic_hex = match read_magic_number(path) {
            Some(m) => format!("{:02X} {:02X} {:02X} {:02X}", m[0], m[1], m[2], m[3]),
            None => "N/A".to_string(),
        };
        // Grab the first 16 bytes to show in the UI for the user's reference
        let magic_hex_16 = match read_file_header_16(path) {
            Some(m) => m.iter().map(|b| format!("{:02X}", b)).collect::<Vec<String>>().join(" "),
            None => "N/A".to_string(),
        };

        let final_format = if detected_name.is_empty() { "Unknown/Text".to_string() } else { detected_name };
        Some(AuthResult { path: path.clone(), extension: ext, detected_format: final_format, magic_hex, magic_hex_16, is_warning })
    } else {
        None
    }
}

/// Helper function to convert an Option<PathBuf> to a displayable string
fn path_buf_to_string(path: &Option<PathBuf>) -> String {
    path.as_ref()
        .and_then(|p| p.to_str())
        .unwrap_or("")
        .to_string()
}

/// The actual file operation logic, moved into its own function
fn perform_replacement(
    replacement_file: Option<PathBuf>,
    files_to_replace: Vec<PathBuf>,
    export_mode: ExportMode,
    copy_to_location: Option<PathBuf>,
    sender: mpsc::Sender<JobMessage>,
) -> ProcessingResult {
    let replacement_file = match replacement_file {
        Some(file) => file,
        None => return ProcessingResult::Error("Replacement file not selected.".to_string()),
    };

    if files_to_replace.is_empty() {
        return ProcessingResult::Error("No files to replace have been selected.".to_string());
    }

    if export_mode == ExportMode::SafeExport && copy_to_location.is_none() {
        return ProcessingResult::Error("A 'copy to' location must be selected.".to_string());
    }

    // Find the common base path for all target files to preserve structure when exporting or backing up.
    let base_path_for_structure = if export_mode == ExportMode::SafeExport || copy_to_location.is_some() {
        find_common_base_path(&files_to_replace)
    } else {
        None
    };

    // If copying or backing up, create a single, neatly-named folder inside the destination to prevent desktop clutter!
    let safe_dest_dir = if export_mode == ExportMode::SafeExport {
        let copy_location = copy_to_location.as_ref().unwrap();
        let folder_name = format!("{}_Replacements", replacement_file.file_stem().unwrap_or_default().to_string_lossy());
        let dir = copy_location.join(folder_name);
        if let Err(e) = fs::create_dir_all(&dir) {
            return ProcessingResult::Error(format!("Failed to create directory {}: {}", dir.display(), e));
        }
        Some(dir)
    } else if export_mode == ExportMode::Overwrite && copy_to_location.is_some() {
        let copy_location = copy_to_location.as_ref().unwrap();
        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let folder_name = format!("Originals_Backup_{}", timestamp);
        let dir = copy_location.join(folder_name);
        if let Err(e) = fs::create_dir_all(&dir) {
            return ProcessingResult::Error(format!("Failed to create backup directory {}: {}", dir.display(), e));
        }
        Some(dir)
    } else {
        None
    };

    // Perform the file operations
    let total = files_to_replace.len();
    for (index, file_to_replace) in files_to_replace.into_iter().enumerate() {
        if let Some(ref export_root_dir) = safe_dest_dir {
            // If we have a common base path, use it to build the sub-directory structure
            let dest_path = if let Some(ref base_path) = base_path_for_structure {
                // This should always succeed if base_path logic is correct
                if let Ok(relative_path) = file_to_replace.strip_prefix(base_path) {
                    let new_dest = export_root_dir.join(relative_path);
                    // Ensure parent directory exists before copying
                    if let Some(parent) = new_dest.parent() {
                        if let Err(e) = fs::create_dir_all(parent) {
                            return ProcessingResult::Error(format!("Failed to create directory {}: {}", parent.display(), e));
                        }
                    }
                    new_dest
                } else {
                    // Fallback to flat structure if stripping prefix fails for some reason
                    let file_name = file_to_replace.file_name().unwrap_or_default();
                    export_root_dir.join(file_name)
                }
            } else {
                // No common base path found, so just dump files in the root of the export folder
                let file_name = file_to_replace.file_name().unwrap_or_default();
                export_root_dir.join(file_name)
            };

            if export_mode == ExportMode::SafeExport {
                if let Err(e) = fs::copy(&replacement_file, &dest_path) {
                    return ProcessingResult::Error(format!("Failed to process {}: {}", file_to_replace.display(), e));
                }
            } else {
                // Overwrite mode with backup
                if let Err(e) = fs::copy(&file_to_replace, &dest_path) {
                    return ProcessingResult::Error(format!("Failed to backup {}: {}", file_to_replace.display(), e));
                }
                if let Err(e) = fs::copy(&replacement_file, &file_to_replace) {
                    return ProcessingResult::Error(format!("Failed to process {}: {}", file_to_replace.display(), e));
                }
            }
        } else {
            if let Err(e) = fs::copy(&replacement_file, &file_to_replace) {
                return ProcessingResult::Error(format!("Failed to process {}: {}", file_to_replace.display(), e));
            }
        }

        // Send progress update back to the main UI thread!
        sender.send(JobMessage::Progress(index + 1, total)).ok();
    }

    ProcessingResult::Success
}

/// Scans dropped files in a background thread to avoid blocking the UI
fn perform_file_scan(
    dropped_files: Vec<egui::DroppedFile>,
    current_master_file: Option<PathBuf>,
    disable_safety_checks: bool,
) -> ScanMessage {
    let mut new_master = None;
    let mut new_targets = Vec::new();
    let mut discarded_count = 0;

    // Step 1: Expand any dropped folders into a flat list of files!
    let mut paths_to_process = Vec::new();
    for file in dropped_files {
        if let Some(path) = file.path {
            if path.is_file() {
                paths_to_process.push(path);
            } else if path.is_dir() {
                collect_files_recursively(&path, &mut paths_to_process);
            }
        }
    }

    // Determine the master file to check against.
    // If one is already set in the UI, use that.
    // If not, the first file from the drop will become the master.
    let mut master_file_for_checks = current_master_file;
    let mut master_file_is_from_this_drop = false;

    if master_file_for_checks.is_none() && !paths_to_process.is_empty() {
        // Take the first file from the list to be the master.
        let first_file = paths_to_process.remove(0);
        master_file_for_checks = Some(first_file);
        master_file_is_from_this_drop = true;
    }

    // If there's still no master file (e.g., an empty folder was dropped), we can't do anything.
    let master_path = match master_file_for_checks {
        Some(path) => path,
        None => {
            return ScanMessage::ScanFinished {
                new_master: None,
                new_targets: vec![],
                discarded_count: 0,
            };
        }
    };

    // If the master file came from this drop, we need to send it back to the UI.
    if master_file_is_from_this_drop {
        new_master = Some(master_path.clone());
    }

    // Get master file properties for comparison
    let top_ext = master_path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let master_magic = read_magic_number(&master_path);

    // Step 2: Process all other files against the master
    for path in paths_to_process {
        // Don't add the master file to the target list if it was dropped again
        if path == master_path {
            continue;
        }

        let mut discard = false;
        if !disable_safety_checks {
            let bottom_ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            if !top_ext.is_empty() && !bottom_ext.is_empty() && top_ext != bottom_ext {
                discard = true;
            } else if let (Some(m_magic), Some(t_magic)) = (master_magic, read_magic_number(&path)) {
                if m_magic != t_magic {
                    discard = true;
                }
            }
        }

        if discard {
            discarded_count += 1;
        } else {
            new_targets.push(path);
        }
    }

    ScanMessage::ScanFinished {
        new_master,
        new_targets,
        discarded_count,
    }
}

/// Performs safety validation in a background thread to avoid blocking the UI
fn perform_validation(
    files_to_replace: Vec<PathBuf>,
    current_master: Option<PathBuf>,
    previous_master: Option<PathBuf>,
    top_file_changed: bool,
    safety_just_enabled: bool,
) -> ValidationMessage {
    let mut validated_targets = files_to_replace.clone();
    let mut new_master = current_master.clone();
    let mut warning = None;

    if let Some(master_path) = &current_master {
        let top_ext = master_path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        let master_magic = read_magic_number(master_path);

        // Scenario 1: The user changed the master file. We must check if it's compatible with the existing list.
        if top_file_changed {
            let mut mismatch_found = false;
            for path in &files_to_replace {
                let bottom_ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                if !top_ext.is_empty() && !bottom_ext.is_empty() && top_ext != bottom_ext {
                    mismatch_found = true;
                    break;
                }
                if let (Some(m_magic), Some(t_magic)) = (master_magic, read_magic_number(path)) {
                    if m_magic != t_magic {
                        mismatch_found = true;
                        break;
                    }
                }
            }

            if mismatch_found {
                // A mismatch was found, so we reject the change and revert the master file.
                new_master = previous_master;
                warning = Some("The new Replacement File does not match the format of the files already in your list!\n\nThe change was rejected to protect your existing file list. If you intentionally want to use a different file type, please clear the bottom list first or disable Safety Checks.".to_string());
                // The target list itself doesn't change in this case, we just return.
                return ValidationMessage::ValidationComplete { validated_targets, new_master, warning };
            }
        }

        // Scenario 2: The user just re-enabled safety checks. We need to purge any invalid files from the list.
        if safety_just_enabled {
            let original_count = validated_targets.len();
            validated_targets.retain(|path| {
                let bottom_ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                if !top_ext.is_empty() && !bottom_ext.is_empty() && top_ext != bottom_ext {
                    return false; // Discard
                }
                if let (Some(m_magic), Some(t_magic)) = (master_magic, read_magic_number(path)) {
                    if m_magic != t_magic {
                        return false; // Discard
                    }
                }
                true // Keep
            });
            let discarded_count = original_count - validated_targets.len();
            if discarded_count > 0 {
                warning = Some(format!("Safety checks were re-enabled. {} incompatible files were automatically removed from the list.", discarded_count));
            }
        }
    }

    ValidationMessage::ValidationComplete { validated_targets, new_master, warning }
}
/// Reads the first 4 bytes of a file to get its "Magic Number" signature
fn read_magic_number(path: &PathBuf) -> Option<[u8; 4]> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 4];
    file.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// Reads up to the first 16 bytes of a file for detailed hex analysis
fn read_file_header_16(path: &PathBuf) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    file.by_ref().take(16).read_to_end(&mut buf).ok()?;
    if buf.is_empty() {
        return None;
    }
    Some(buf)
}

/// Layer 2: The Signature Search (Reads first 2 KB)
/// Distinguishes between "Container" files (XML vs SVG, text formats)
fn layer2_signature_search(path: &PathBuf) -> Option<(String, String)> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 2048];
    let bytes_read = file.read(&mut buf).ok()?;
    if bytes_read == 0 {
        return None;
    }

    let content = String::from_utf8_lossy(&buf[..bytes_read]);
    let content_lower = content.to_lowercase();

    if content_lower.contains("<svg") {
        return Some(("svg".to_string(), "image/svg+xml (SVG Vector Image)".to_string()));
    } else if content_lower.contains("<?xml") {
        return Some(("xml".to_string(), "text/xml (XML Document)".to_string()));
    } else if content_lower.contains("<html") || content_lower.contains("<!doctype html") {
        return Some(("html".to_string(), "text/html (HTML Document)".to_string()));
    }

    None
}

/// Layer 3: The Deep Dive (Scans the end of the file)
/// Used for corrupted files or finding "trailers" (like hidden ZIPs at the end of an EXE)
fn layer3_deep_dive(path: &PathBuf) -> Option<(String, String)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();

    if len < 22 {
        return None;
    }

    // Read up to the last 64 KB (max size of zip EOCD comment is 64KB)
    let read_size = std::cmp::min(len, 65536);
    file.seek(SeekFrom::End(-(read_size as i64))).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;

    // Search backwards for PK\x05\x06 (ZIP End of Central Directory)
    for i in (0..buf.len().saturating_sub(3)).rev() {
        if buf[i] == 0x50 && buf[i+1] == 0x4B && buf[i+2] == 0x05 && buf[i+3] == 0x06 {
            return Some(("zip".to_string(), "application/zip (ZIP Archive Trailer)".to_string()));
        }
    }

    None
}

/// Finds the deepest common directory for a slice of paths.
fn find_common_base_path(paths: &[PathBuf]) -> Option<PathBuf> {
    if paths.len() < 2 {
        // If there's 0 or 1 path, the "common" base is just the file's own parent directory.
        return paths.first().and_then(|p| p.parent()).map(|p| p.to_path_buf());
    }

    let mut common_path = match paths[0].parent() {
        Some(p) => p.to_path_buf(),
        None => return None, // First path has no parent (e.g., is a root like "C:\")
    };

    // Iteratively shorten the common_path until all other paths are children of it.
    for path in paths.iter().skip(1) {
        while !path.starts_with(&common_path) {
            if !common_path.pop() {
                // We've reached the root and there's still no commonality (e.g., C:\ vs D:\).
                return None;
            }
        }
    }
    Some(common_path)
}
/// Recursively collects all files within a directory tree
fn collect_files_recursively(dir: &PathBuf, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                files.push(path);
            } else if path.is_dir() {
                collect_files_recursively(&path, files);
            }
        }
    }
}
