use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use eframe::egui::{self, Color32, RichText};
use crate::ignore::{conda_ignore_rules, IgnoreEngine, DEFAULT_IGNORE_FILENAME};
use crate::migrator::{migrate_environment, preview_environment, MigrationResult, PythonPolicy};
use crate::platform::{current_os, disk_free_space_gb, format_bytes};
use crate::python_detector::{discover_all_pythons, PythonKind, PythonRuntime};
use crate::uv::{find_uv, get_install_guide, UvInfo};
use crate::venv_detector::{scan_with_inspection, Inspection, VenvInfo};

const DEFAULT_IGNORE_TEXT: &str = "# uv-migrator Do Not Touch list\n**/miniconda3/**\n**/anaconda3/**\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    DetectUv,
    Scan,
    ReviewIgnore,
    UpgradeOptions,
    Migrate,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeMode {
    SameVersion,
    LatestPatch,
    NewestOverall,
    Custom,
}

pub enum BackgroundEvent {
    ScanComplete {
        generation: u64,
        input: ScanInput,
        result: Result<(Vec<PythonRuntime>, Vec<VenvInfo>), String>,
    },
    MigrationProgress {
        index: usize,
        total: usize,
        rel_path: String,
        result: MigrationResult,
    },
    MigrationComplete {
        results: Vec<MigrationResult>,
        initial_free_gb: Option<f64>,
        final_free_gb: Option<f64>,
    },
}

#[derive(Debug, Clone)]
pub struct ScanInput {
    root: String,
    ignore_file: PathBuf,
    ignore_text: String,
}

pub struct UvMigratorApp {
    pub current_step: Step,
    pub uv_info: Option<UvInfo>,
    pub search_path: String,

    // Step 1 & 2 data
    pub pythons: Vec<PythonRuntime>,
    pub venvs: Vec<VenvInfo>,
    pub selected_venvs: HashSet<PathBuf>,
    pub ignore_text: String,
    ignore_file: PathBuf,
    pub is_scanning: bool,
    scan_generation: u64,
    scanned_input: Option<ScanInput>,
    scan_error: Option<String>,

    // Step 3 options
    pub upgrade_mode: UpgradeMode,
    pub custom_python: String,
    pub dry_run: bool,
    pub auto_install_python: bool,

    // Step 4 migration progress
    pub is_migrating: bool,
    pub progress_fraction: f32,
    pub progress_text: String,
    pub migration_results: Vec<MigrationResult>,
    pub initial_free_gb: Option<f64>,
    pub final_free_gb: Option<f64>,
    pub activity_logs: Vec<String>,

    // Channels
    sender: Sender<BackgroundEvent>,
    receiver: Receiver<BackgroundEvent>,
}

impl UvMigratorApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let default_path = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string());

        let mut app = Self::with_uv(default_path, find_uv());
        if app.uv_info.is_some() {
            app.scan_from_disk();
        }
        app
    }

    fn with_uv(search_path: String, uv: Option<UvInfo>) -> Self {
        let (sender, receiver) = channel();
        Self {
            current_step: if uv.is_some() { Step::Scan } else { Step::DetectUv },
            uv_info: uv,
            search_path,
            pythons: Vec::new(),
            venvs: Vec::new(),
            selected_venvs: HashSet::new(),
            ignore_text: String::new(),
            ignore_file: PathBuf::new(),
            is_scanning: false,
            scan_generation: 0,
            scanned_input: None,
            scan_error: None,
            upgrade_mode: UpgradeMode::LatestPatch,
            custom_python: "3.14".to_string(),
            dry_run: false,
            auto_install_python: true,
            is_migrating: false,
            progress_fraction: 0.0,
            progress_text: "Ready to start".to_string(),
            migration_results: Vec::new(),
            initial_free_gb: None,
            final_free_gb: None,
            activity_logs: Vec::new(),
            sender,
            receiver,
        }
    }

    pub fn check_uv(&mut self) {
        self.uv_info = find_uv();
        if self.uv_info.is_some() && self.current_step == Step::DetectUv {
            self.current_step = Step::Scan;
            self.scan_from_disk();
        }
    }

    fn scan_from_disk(&mut self) {
        if self.is_migrating {
            return;
        }
        match IgnoreEngine::read_default(Path::new(&self.search_path)) {
            Ok(file) => {
                let (path, text) = file.unwrap_or_else(|| {
                    (
                        Path::new(&self.search_path).join(DEFAULT_IGNORE_FILENAME),
                        DEFAULT_IGNORE_TEXT.to_string(),
                    )
                });
                self.ignore_file = path;
                self.ignore_text = text;
                self.start_background_scan();
            }
            Err(error) => {
                self.ignore_text.clear();
                self.invalidate_scan(error);
            }
        }
    }

    fn invalidate_scan(&mut self, error: String) {
        self.scan_generation += 1;
        self.is_scanning = false;
        self.scan_error = Some(error);
    }

    fn save_ignore_text(&mut self) {
        let ignore_file = Path::new(&self.search_path).join(DEFAULT_IGNORE_FILENAME);
        let result = IgnoreEngine::from_text(&ignore_file, &self.ignore_text).and_then(|_| {
            std::fs::write(&ignore_file, &self.ignore_text)
                .map_err(|error| format!("Could not write {}: {error}", ignore_file.display()))
        });
        match result {
            Ok(()) => {
                self.ignore_file = ignore_file;
                self.start_background_scan();
            }
            Err(error) => self.invalidate_scan(error),
        }
    }

    pub fn start_background_scan(&mut self) {
        if self.is_migrating {
            return;
        }
        self.is_scanning = true;
        self.scan_error = None;
        self.scan_generation += 1;
        let generation = self.scan_generation;
        let input = ScanInput {
            root: self.search_path.clone(),
            ignore_file: self.ignore_file.clone(),
            ignore_text: self.ignore_text.clone(),
        };
        let uv = self.uv_info.clone();
        let tx = self.sender.clone();

        std::thread::spawn(move || {
            let result = (|| {
                let path = std::path::absolute(&input.root).map_err(|error| format!("Invalid search root: {error}"))?;
                if !path.is_dir() {
                    return Err(format!("Search root is not a directory: {}", path.display()));
                }
                let ignore_engine = IgnoreEngine::from_text(&input.ignore_file, &input.ignore_text)?;
                let venvs = scan_with_inspection(&path, &ignore_engine, true, Inspection::Detailed)?;
                let pythons = discover_all_pythons(uv.as_ref());
                Ok((pythons, venvs))
            })();
            let _ = tx.send(BackgroundEvent::ScanComplete { generation, input, result });
        });
    }

    fn scan_is_current(&self) -> bool {
        !self.is_scanning
            && self.scan_error.is_none()
            && self.scanned_input.as_ref().is_some_and(|input| {
                input.root == self.search_path && input.ignore_text == self.ignore_text && input.ignore_file == self.ignore_file
            })
    }

    fn can_start_migration(&self) -> bool {
        !self.is_migrating && self.scan_is_current() && self.uv_info.is_some() && !self.selected_venvs.is_empty()
    }

    pub fn start_migration(&mut self) {
        if !self.can_start_migration() {
            return;
        }
        match IgnoreEngine::read_default(Path::new(&self.search_path)) {
            Ok(Some((path, text))) if path == self.ignore_file && text == self.ignore_text => {}
            Ok(None) if self.ignore_text == DEFAULT_IGNORE_TEXT => {}
            Ok(_) => {
                self.scan_from_disk();
                return;
            }
            Err(error) => {
                self.invalidate_scan(error);
                return;
            }
        }
        let Some(uv) = self.uv_info.clone() else { return };
        let policy = match self.upgrade_mode {
            UpgradeMode::LatestPatch => PythonPolicy::LatestPatch,
            UpgradeMode::NewestOverall => PythonPolicy::Latest,
            UpgradeMode::SameVersion => PythonPolicy::Same,
            UpgradeMode::Custom => match self.custom_python.trim().parse() {
                Ok(policy @ PythonPolicy::Exact(_)) => policy,
                _ => return,
            },
        };

        let venvs_to_migrate: Vec<VenvInfo> = self
            .venvs
            .iter()
            .filter(|v| self.selected_venvs.contains(&v.venv_dir) && !v.is_ignored)
            .cloned()
            .collect();
        if venvs_to_migrate.is_empty() {
            return;
        }
        self.is_migrating = true;
        self.current_step = Step::Migrate;
        self.progress_fraction = 0.0;
        self.progress_text = "Preparing migration...".to_string();
        self.activity_logs.clear();
        self.migration_results.clear();
        self.initial_free_gb = None;
        self.final_free_gb = None;

        let root = PathBuf::from(&self.search_path);
        let pythons = self.pythons.clone();
        let dry_run = self.dry_run;
        let allow_downloads = self.auto_install_python;
        let tx = self.sender.clone();

        std::thread::spawn(move || {
            let initial_free = if dry_run { None } else { disk_free_space_gb(&root) };
            let total = venvs_to_migrate.len();
            let mut results = Vec::new();

            for (idx, venv) in venvs_to_migrate.iter().enumerate() {
                let res = if dry_run {
                    preview_environment(venv, &pythons, &policy, true)
                } else {
                    migrate_environment(venv, &uv, &pythons, &policy, allow_downloads)
                };
                results.push(res.clone());

                let _ = tx.send(BackgroundEvent::MigrationProgress {
                    index: idx + 1,
                    total,
                    rel_path: res.venv_dir.display().to_string(),
                    result: res,
                });
            }

            let final_free = if dry_run { None } else { disk_free_space_gb(&root) };
            let _ = tx.send(BackgroundEvent::MigrationComplete {
                results,
                initial_free_gb: initial_free,
                final_free_gb: final_free,
            });
        });
    }

    pub fn poll_events(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                BackgroundEvent::ScanComplete { generation, input, result } => {
                    if generation != self.scan_generation {
                        continue;
                    }
                    self.is_scanning = false;
                    if input.root != self.search_path || input.ignore_text != self.ignore_text || input.ignore_file != self.ignore_file {
                        continue;
                    }
                    let (pythons, venvs) = match result {
                        Ok(scan) => scan,
                        Err(error) => {
                            self.scan_error = Some(error);
                            continue;
                        }
                    };
                    self.pythons = pythons;
                    let same_root = self.scanned_input.as_ref().is_some_and(|previous| previous.root == input.root);
                    let previous_paths: HashSet<_> = self.venvs.iter().map(|venv| &venv.venv_dir).collect();
                    let mut selected = HashSet::new();
                    for v in &venvs {
                        if !v.is_ignored && (!same_root || !previous_paths.contains(&v.venv_dir) || self.selected_venvs.contains(&v.venv_dir)) {
                            selected.insert(v.venv_dir.clone());
                        }
                    }
                    self.selected_venvs = selected;
                    self.venvs = venvs;
                    self.scanned_input = Some(input);
                }
                BackgroundEvent::MigrationProgress { index, total, rel_path, result } => {
                    self.progress_fraction = index as f32 / total.max(1) as f32;
                    self.progress_text = format!("[{}/{}] Migrated {}", index, total, rel_path);
                    let log_line = if result.success && result.dry_run {
                        format!("✔ [{}/{}] {} (dry run: {} packages to rebuild and verify)", index, total, rel_path, result.package_count.unwrap_or_default())
                    } else if result.success {
                        let note = result.error.as_deref().map(|e| format!(" - {}", e)).unwrap_or_default();
                        format!("✔ [{}/{}] {} ({} -> {}, {} packages verified){}", index, total, rel_path, format_bytes(result.old_size_bytes.unwrap_or_default()), format_bytes(result.new_size_bytes.unwrap_or_default()), result.package_count.unwrap_or_default(), note)
                    } else {
                        format!("✖ [{}/{}] {} - Error: {}", index, total, rel_path, result.error.as_deref().unwrap_or("Unknown"))
                    };
                    self.activity_logs.push(log_line);
                }
                BackgroundEvent::MigrationComplete { results, initial_free_gb, final_free_gb } => {
                    self.is_migrating = false;
                    self.migration_results = results;
                    self.initial_free_gb = initial_free_gb;
                    self.final_free_gb = final_free_gb;
                    self.current_step = Step::Completed;
                }
            }
        }
    }
}

impl eframe::App for UvMigratorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events();

        egui::TopBottomPanel::top("top_header").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading(RichText::new("⚡ uv-migrator").strong().color(Color32::from_rgb(0, 210, 255)));
                ui.label(RichText::new("Virtual Environment Migrator").color(Color32::GRAY));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(ref uv) = self.uv_info {
                        ui.label(
                            RichText::new(format!("✔ uv {}", uv.version))
                                .color(Color32::from_rgb(50, 220, 100))
                                .strong(),
                        );
                    } else {
                        ui.label(RichText::new("✖ uv missing").color(Color32::from_rgb(255, 80, 80)).strong());
                    }
                });
            });
            ui.add_space(6.0);

            // Step Navigation Tabs (if UV is available)
            if self.uv_info.is_some() && !self.is_migrating {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.current_step, Step::Scan, "1. Select & Scan");
                    ui.label("➔");
                    ui.selectable_value(&mut self.current_step, Step::ReviewIgnore, "2. Review & Ignore");
                    ui.label("➔");
                    ui.selectable_value(&mut self.current_step, Step::UpgradeOptions, "3. Upgrade Strategy");
                    ui.label("➔");
                    ui.selectable_value(&mut self.current_step, Step::Migrate, "4. Migration Progress");
                });
                ui.add_space(4.0);
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(error) = &self.scan_error {
                ui.colored_label(Color32::LIGHT_RED, error);
            }
            if self.uv_info.is_none() || self.current_step == Step::DetectUv {
                self.render_missing_uv_screen(ui, ctx);
            } else {
                match self.current_step {
                    Step::Scan => self.render_scan_step(ui),
                    Step::ReviewIgnore => self.render_ignore_step(ui),
                    Step::UpgradeOptions => self.render_upgrade_step(ui),
                    Step::Migrate | Step::Completed => self.render_migrate_step(ui),
                    Step::DetectUv => self.render_missing_uv_screen(ui, ctx),
                }
            }
        });

        // Request continuous repaint if an async job is active
        if self.is_scanning || self.is_migrating {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

impl UvMigratorApp {
    fn render_missing_uv_screen(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.vertical_centered(|ui| {
            ui.add_space(30.0);
            ui.label(RichText::new("⚠️ 'uv' Was Not Detected on Your System").size(22.0).strong().color(Color32::from_rgb(255, 180, 0)));
            ui.add_space(10.0);
            ui.label(RichText::new("uv is an extremely fast Python package manager and virtualenv engine from Astral.\nuv-migrator uses uv to hardlink virtual environments and reclaim gigabytes of disk space.").color(Color32::LIGHT_GRAY));

            ui.add_space(20.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_max_width(650.0);
                ui.label(RichText::new(format!("Recommended Install Command for {}:", current_os())).strong());
                ui.add_space(6.0);

                let guide = get_install_guide();
                ui.monospace(&guide);
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if ui.button("📋 Copy Command").clicked() {
                        ctx.output_mut(|o| o.copied_text = guide.clone());
                    }
                    if ui.button("🌐 Open Official Installation Guide").clicked() {
                        ctx.open_url(egui::OpenUrl::new_tab("https://docs.astral.sh/uv/getting-started/installation/"));
                    }
                });
            });

            ui.add_space(25.0);
            if ui.button(RichText::new("🔄 Re-Check UV Installation").size(16.0).strong().color(Color32::WHITE)).clicked() {
                self.check_uv();
            }
        });
    }

    fn render_scan_step(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.heading("Step 1: Select Folder & Scan");
        ui.label("Select a root directory to discover virtual environments and installed Python runtimes.");
        ui.add_space(10.0);

        // Path input + browse button
        ui.horizontal(|ui| {
            ui.label("Target Folder:");
            let text_edit = ui.text_edit_singleline(&mut self.search_path);
            if text_edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                self.scan_from_disk();
            }

            if ui.button("📁 Browse...").clicked() {
                if let Some(folder) = rfd::FileDialog::new().set_directory(&self.search_path).pick_folder() {
                    self.search_path = folder.to_string_lossy().to_string();
                    self.scan_from_disk();
                }
            }

            if ui.button(RichText::new("🔍 Scan").strong()).clicked() && !self.is_scanning {
                self.scan_from_disk();
            }
        });

        ui.add_space(15.0);

        if self.is_scanning {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Scanning directories and discovering runtimes...").italics());
            });
        } else {
            // Summary cards
            ui.horizontal(|ui| {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_width(200.0);
                    ui.label(RichText::new(format!("{} Environments Found", self.venvs.len())).strong().size(16.0));
                    let total_size: u64 = self.venvs.iter().filter_map(|v| v.size_bytes).sum();
                    ui.label(format!("Total Footprint: {}", format_bytes(total_size)));
                });

                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_width(200.0);
                    ui.label(RichText::new(format!("{} Python Runtimes", self.pythons.len())).strong().size(16.0));
                    let uv_count = self.pythons.iter().filter(|p| matches!(p.kind, PythonKind::UvManaged)).count();
                    ui.label(format!("{} uv-managed, {} system/conda", uv_count, self.pythons.len() - uv_count));
                });
            });

            ui.add_space(15.0);

            // Collapsible Python runtimes list
            egui::CollapsingHeader::new(RichText::new(format!("Detected Python Runtimes ({})", self.pythons.len())).strong())
                .default_open(false)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                        for p in &self.pythons {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&p.version).strong());
                                ui.label(RichText::new(format!("({})", p.kind)).color(Color32::LIGHT_BLUE));
                                ui.label(RichText::new(p.path.display().to_string()).color(Color32::GRAY));
                            });
                        }
                    });
                });

            ui.add_space(20.0);
            if !self.venvs.is_empty() {
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Next: Review & Configure Ignores ➔").size(15.0).strong()).clicked() {
                        self.current_step = Step::ReviewIgnore;
                    }
                });
            }
        }
    }

    fn render_ignore_step(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.heading("Step 2: Review Environments & 'Do Not Touch' List");
        ui.label("Toggle virtual environments to migrate and configure exclusion rules.");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            if ui.button("Select All Active").clicked() {
                for v in &self.venvs {
                    if !v.is_ignored {
                        self.selected_venvs.insert(v.venv_dir.clone());
                    }
                }
            }
            if ui.button("Deselect All").clicked() {
                self.selected_venvs.clear();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let count = self.selected_venvs.len();
                ui.label(RichText::new(format!("{} Selected for Migration", count)).strong().color(Color32::from_rgb(0, 200, 255)));
            });
        });

        ui.add_space(8.0);

        // Split view: Environment Table and Live Ignore Editor
        ui.columns(2, |cols| {
            // Left Column: Environment Table
            cols[0].group(|ui| {
                ui.label(RichText::new("Virtual Environments").strong());
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for v in &self.venvs {
                        let mut is_sel = self.selected_venvs.contains(&v.venv_dir);
                        ui.horizontal(|ui| {
                            if ui.add_enabled(!v.is_ignored, egui::Checkbox::new(&mut is_sel, "")).changed() {
                                if is_sel {
                                    self.selected_venvs.insert(v.venv_dir.clone());
                                } else {
                                    self.selected_venvs.remove(&v.venv_dir);
                                }
                            }

                            let name = Path::new(&v.rel_path).join(v.venv_dir.file_name().unwrap_or_default());
                            let name = name.display().to_string();

                            if v.is_ignored {
                                ui.label(RichText::new(name).strikethrough().color(Color32::from_rgb(255, 140, 0)));
                                ui.label(RichText::new("[IGNORED]").color(Color32::from_rgb(255, 140, 0)).small());
                            } else {
                                ui.label(RichText::new(name).strong());
                            }

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let size = v.size_bytes.map_or_else(|| "unmeasured".to_string(), format_bytes);
                                ui.label(RichText::new(size).color(Color32::LIGHT_GRAY));
                                ui.label(RichText::new(&v.python_version_raw).color(Color32::LIGHT_BLUE));
                            });
                        });
                    }
                });
            });

            // Right Column: Live Ignore File Editor
            cols[1].group(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(".uv-migrator-ignore").strong());
                    if ui.button("💾 Save").clicked() {
                        self.save_ignore_text();
                    }
                });

                ui.add_space(4.0);
                ui.add(
                    egui::TextEdit::multiline(&mut self.ignore_text)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(14)
                        .desired_width(f32::INFINITY),
                );

                ui.horizontal(|ui| {
                    if ui.button("+ Protect Conda").clicked() {
                        for rule in conda_ignore_rules(&self.pythons) {
                            if !self.ignore_text.lines().any(|line| line.trim() == rule) {
                                if !self.ignore_text.is_empty() && !self.ignore_text.ends_with('\n') {
                                    self.ignore_text.push('\n');
                                }
                                self.ignore_text.push_str(&rule);
                                self.ignore_text.push('\n');
                            }
                        }
                    }
                });
            });
        });

        ui.add_space(15.0);
        ui.horizontal(|ui| {
            if ui.button("⬅ Back").clicked() {
                self.current_step = Step::Scan;
            }
            if ui.button(RichText::new("Next: Choose Upgrade Strategy ➔").size(15.0).strong()).clicked() {
                self.current_step = Step::UpgradeOptions;
            }
        });
    }

    fn render_upgrade_step(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        ui.heading("Step 3: Python Version & Upgrade Strategy");
        ui.label("Select how Python runtime versions should be handled during migration.");
        ui.add_space(15.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_max_width(700.0);

            ui.radio_value(
                &mut self.upgrade_mode,
                UpgradeMode::LatestPatch,
                RichText::new("Upgrade to Latest Installed Patch").strong(),
            );
            ui.label(RichText::new("  Selects the highest installed patch in each environment’s Python series.").color(Color32::LIGHT_GRAY));
            ui.add_space(8.0);

            ui.radio_value(
                &mut self.upgrade_mode,
                UpgradeMode::SameVersion,
                RichText::new("Keep Current Python Major.Minor Series").strong(),
            );
            ui.label(RichText::new("  Matches the exact major.minor version of each existing environment.").color(Color32::LIGHT_GRAY));
            ui.add_space(8.0);

            ui.radio_value(
                &mut self.upgrade_mode,
                UpgradeMode::NewestOverall,
                RichText::new("Upgrade All Environments to Newest System Python").strong(),
            );
            ui.label(RichText::new("  Upgrades all migratable projects to the newest available Python overall (e.g. 3.14.7).").color(Color32::LIGHT_GRAY));
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.radio_value(&mut self.upgrade_mode, UpgradeMode::Custom, RichText::new("Custom Python Version:").strong());
                if self.upgrade_mode == UpgradeMode::Custom {
                    ui.text_edit_singleline(&mut self.custom_python);
                }
            });
        });

        ui.add_space(15.0);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_max_width(700.0);
            ui.checkbox(&mut self.dry_run, RichText::new("Dry Run Mode (Simulation only, do not modify files)").strong());
            ui.checkbox(&mut self.auto_install_python, "Auto-download standalone Python via uv if target version is not installed");
        });

        ui.add_space(20.0);
        ui.horizontal(|ui| {
            if ui.button("⬅ Back").clicked() {
                self.current_step = Step::ReviewIgnore;
            }

            let sel_count = self.selected_venvs.len();
            let action_btn = if self.dry_run {
                RichText::new(format!("Simulate Migration ({} Environments)", sel_count)).size(16.0).strong().color(Color32::from_rgb(255, 200, 0))
            } else {
                RichText::new(format!("Start Migration ({} Environments) 🚀", sel_count)).size(16.0).strong().color(Color32::from_rgb(0, 255, 180))
            };

            let valid_python = self.upgrade_mode != UpgradeMode::Custom
                || matches!(self.custom_python.trim().parse(), Ok(PythonPolicy::Exact(_)));
            if ui.add_enabled(valid_python && self.can_start_migration(), egui::Button::new(action_btn)).clicked() {
                self.start_migration();
            }
            if !valid_python {
                ui.label("Enter a Python version such as 3.12 or 3.12.4.");
            } else if !self.scan_is_current() {
                ui.label("Scan the current folder and save any ignore changes before migrating.");
            }
        });
    }

    fn render_migrate_step(&mut self, ui: &mut egui::Ui) {
        ui.add_space(10.0);
        if self.is_migrating {
            ui.heading("Step 4: Migrating Virtual Environments...");
        } else {
            ui.heading(RichText::new("Migration Completed! 🎉").color(Color32::from_rgb(50, 230, 100)));
        }

        ui.add_space(10.0);

        // Progress Bar
        ui.add(egui::ProgressBar::new(self.progress_fraction).show_percentage().animate(self.is_migrating));
        ui.label(RichText::new(&self.progress_text).strong());

        ui.add_space(15.0);

        // Results Card if Completed
        if !self.is_migrating && !self.migration_results.is_empty() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let old_total: u64 = self.migration_results.iter().filter_map(|r| r.old_size_bytes).sum();
                    let new_total: u64 = self.migration_results.iter().filter_map(|r| r.new_size_bytes).sum();
                    let success_count = self.migration_results.iter().filter(|r| r.success).count();

                    ui.vertical(|ui| {
                        ui.label(RichText::new(format!("Processed: {}/{}", success_count, self.migration_results.len())).strong().size(15.0));
                        ui.label(format!("Old Footprint: {}", format_bytes(old_total)));
                        if self.migration_results.iter().any(|r| r.dry_run) {
                            ui.label("New Footprint: not measured in a dry run");
                        } else {
                            ui.label(format!("New Footprint: {} (apparent size)", format_bytes(new_total)))
                                .on_hover_text("Counts files uv hardlinks from its cache in full. The change in free space is the real figure.");
                        }
                    });

                    ui.add_space(30.0);

                    if let (Some(init_f), Some(fin_f)) = (self.initial_free_gb, self.final_free_gb) {
                        let diff = fin_f - init_f;
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Disk Space Reclaimed:").strong().size(15.0));
                            if diff >= 0.0 {
                                ui.label(RichText::new(format!("+{:.2} GB Free Space", diff)).size(18.0).strong().color(Color32::from_rgb(0, 240, 120)));
                            } else {
                                ui.label(format!("{:.2} GB", diff));
                            }
                        });
                    }
                });
            });
            ui.add_space(10.0);
        }

        // Live Log Box
        ui.label(RichText::new("Activity Log:").strong());
        egui::ScrollArea::vertical().max_height(200.0).stick_to_bottom(true).show(ui, |ui| {
            for log in &self.activity_logs {
                if log.starts_with('✔') {
                    ui.label(RichText::new(log).color(Color32::from_rgb(80, 220, 120)));
                } else if log.starts_with('✖') {
                    ui.label(RichText::new(log).color(Color32::from_rgb(255, 90, 90)));
                } else {
                    ui.label(RichText::new(log).color(Color32::LIGHT_GRAY));
                }
            }
        });

        ui.add_space(15.0);
        if !self.is_migrating {
            ui.horizontal(|ui| {
                if ui.button("🔄 Start New Scan").clicked() {
                    self.current_step = Step::Scan;
                    self.scan_from_disk();
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> UvMigratorApp {
        UvMigratorApp::with_uv("root".into(), Some(UvInfo { path: "uv".into(), version: String::new() }))
    }

    fn environment(path: &str, ignored: bool) -> VenvInfo {
        VenvInfo {
            project_dir: PathBuf::from("root/project"),
            venv_dir: PathBuf::from(path),
            rel_path: "project".into(),
            python_version_raw: "3.12.4".into(),
            parsed_version: Some((3, 12, 4)),
            is_uv_managed: false,
            size_bytes: None,
            has_pyproject: false,
            has_requirements: false,
            has_setup_py: false,
            has_uv_lock: false,
            is_ignored: ignored,
            installed_packages: None,
        }
    }

    fn finish_scan(app: &mut UvMigratorApp, generation: u64, result: Result<Vec<VenvInfo>, String>) {
        app.sender.send(BackgroundEvent::ScanComplete {
            generation,
            input: ScanInput {
                root: app.search_path.clone(),
                ignore_file: app.ignore_file.clone(),
                ignore_text: app.ignore_text.clone(),
            },
            result: result.map(|venvs| (Vec::new(), venvs)),
        }).unwrap();
        app.poll_events();
    }

    #[test]
    fn sibling_selections_stay_independent_after_rescan() {
        let mut app = app();
        let left = environment("root/project/.venv", false);
        let right = environment("root/project/venv", false);
        let ignored = environment("root/project/env", true);
        let venvs = vec![left.clone(), right.clone(), ignored.clone()];
        finish_scan(&mut app, 0, Ok(venvs.clone()));
        assert_eq!(app.selected_venvs.len(), 2);
        assert!(!app.selected_venvs.contains(&ignored.venv_dir));
        app.selected_venvs.remove(&left.venv_dir);
        app.scan_generation = 1;
        finish_scan(&mut app, 1, Ok(venvs));
        assert_eq!(app.selected_venvs, HashSet::from([right.venv_dir]));
    }

    #[test]
    fn stale_and_failed_scans_cannot_start_a_migration() {
        let mut app = app();
        app.scan_generation = 2;
        app.is_scanning = true;
        finish_scan(&mut app, 1, Ok(vec![environment("root/project/.venv", false)]));
        assert!(app.is_scanning);
        assert!(app.venvs.is_empty());
        finish_scan(&mut app, 2, Err("invalid ignore rule".into()));
        assert!(!app.is_scanning);
        assert_eq!(app.scan_error.as_deref(), Some("invalid ignore rule"));
        assert!(!app.can_start_migration());

        app.scan_generation = 3;
        app.scan_error = None;
        finish_scan(&mut app, 3, Ok(vec![environment("root/project/.venv", false)]));
        assert!(app.can_start_migration());
        app.ignore_text = "**/project/**".to_string();
        assert!(!app.can_start_migration());
        app.ignore_text.clear();
        app.search_path = "new-root".into();
        assert!(!app.can_start_migration());
    }

    #[test]
    fn active_migration_blocks_a_second_job_before_changing_state() {
        let mut app = app();
        finish_scan(&mut app, 0, Ok(vec![environment("root/project/.venv", false)]));
        assert!(app.can_start_migration());
        app.is_migrating = true;
        app.progress_fraction = 0.5;
        app.start_migration();
        app.start_background_scan();
        assert_eq!(app.progress_fraction, 0.5);
        assert!(!app.is_scanning);
        assert_eq!(app.scan_generation, 0);
    }
}
