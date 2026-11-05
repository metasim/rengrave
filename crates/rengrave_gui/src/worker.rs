// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Simeon H.K. Fitch

//! Runs layout and toolpath calculations on a background thread.

use rengrave_core::{
    CarvePath, CarvePoint, CleanupBit, CleanupPaths, CutType, Design, Error, FontCatalog,
    FontCoverage, Job, Progress, Settings,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// A unit of work for the background thread.
enum Command {
    Calculate {
        generation: u64,
        settings: Box<Settings>,
        carve: bool,
        reload: bool,
        progress: Progress,
    },
    Cleanup {
        generation: u64,
        request: u64,
        settings: Box<Settings>,
        design: Arc<Design>,
    },
}

/// Tool positions for the straight bit and for the V-bit.
pub type CleanupResult = (Arc<Vec<CarvePoint>>, Arc<Vec<CarvePoint>>);

/// A result from the background thread.
pub enum Update {
    /// The outline is positioned.
    Layout {
        /// The request this answers.
        generation: u64,
        /// The positioned outline.
        design: Arc<Design>,
        /// Non-fatal notes from loading the source.
        warnings: Vec<String>,
        /// The characters the font has, for choosing a page of it.
        coverage: Arc<FontCoverage>,
    },
    /// The V-carve toolpath is finished.
    Carve {
        /// The request this answers.
        generation: u64,
        /// The outline, as sorted for carving.
        design: Arc<Design>,
        /// The toolpath.
        carve: Arc<CarvePath>,
    },
    /// A cleanup calculation stopped.
    Cleanup {
        /// The calculation whose design this used.
        generation: u64,
        /// The cleanup request this answers.
        request: u64,
        /// Paths for the straight bit and for the V-bit, or why there are none.
        paths: Result<CleanupResult, String>,
    },
    /// The calculation stopped.
    Stopped {
        /// The request this answers.
        generation: u64,
        /// Why, or `None` if it finished normally or was canceled.
        error: Option<String>,
    },
}

/// Handle to the background thread.
pub struct Worker {
    commands: Sender<Command>,
    updates: Receiver<Update>,
    fraction: Arc<AtomicU32>,
    active: Mutex<Option<Progress>>,
}

impl Worker {
    /// Starts the thread, which loads fonts from `fonts`. `notify` is called from it whenever there is something new to show.
    pub fn spawn(fonts: Arc<FontCatalog>, notify: impl Fn() + Send + Sync + 'static) -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (update_tx, updates) = mpsc::channel();
        let fraction = Arc::new(AtomicU32::new(0));
        let thread = ThreadState {
            commands: command_rx,
            updates: update_tx,
            fraction: fraction.clone(),
            notify: Arc::new(notify),
            fonts,
            job: None,
            coverage: Arc::default(),
        };
        std::thread::Builder::new()
            .name("rengrave-worker".into())
            .spawn(move || thread.run())
            .expect("the worker thread can be spawned");
        Self {
            commands,
            updates,
            fraction,
            active: Mutex::new(None),
        }
    }

    /// Requests a new calculation, canceling any that is running. With `reload`, the font or
    /// image is read again even if the settings still name the same file.
    pub fn calculate(&self, generation: u64, settings: Settings, carve: bool, reload: bool) {
        let progress = Progress::none();
        if let Ok(mut active) = self.active.lock()
            && let Some(old) = active.replace(progress.clone())
        {
            old.cancel();
        }
        self.fraction.store(0, Ordering::Relaxed);
        // The thread only exits when this handle is dropped, so sending cannot fail here.
        let _ = self.commands.send(Command::Calculate {
            generation,
            settings: Box::new(settings),
            carve,
            reload,
            progress,
        });
    }

    /// Requests cleanup paths for a finished design.
    pub fn cleanup(&self, generation: u64, request: u64, settings: Settings, design: Arc<Design>) {
        let _ = self.commands.send(Command::Cleanup {
            generation,
            request,
            settings: Box::new(settings),
            design,
        });
    }

    /// Stops the running calculation.
    pub fn cancel(&self) {
        if let Ok(active) = self.active.lock()
            && let Some(progress) = active.as_ref()
        {
            progress.cancel();
        }
    }

    /// Completed fraction of the running calculation.
    pub fn fraction(&self) -> f32 {
        self.fraction.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// Returns the next available result.
    pub fn poll(&self) -> Option<Update> {
        self.updates.try_recv().ok()
    }
}

struct ThreadState {
    commands: Receiver<Command>,
    updates: Sender<Update>,
    fraction: Arc<AtomicU32>,
    notify: Arc<dyn Fn() + Send + Sync>,
    fonts: Arc<FontCatalog>,
    job: Option<Job>,
    /// The coverage of the font of `job`.
    coverage: Arc<FontCoverage>,
}

impl ThreadState {
    fn run(mut self) {
        while let Ok(first) = self.commands.recv() {
            // A new calculation supersedes everything queued before it.
            let mut queue = vec![first];
            queue.extend(self.commands.try_iter());
            let start = queue
                .iter()
                .rposition(|c| matches!(c, Command::Calculate { .. }))
                .unwrap_or(0);
            for command in queue.drain(start..) {
                match command {
                    Command::Calculate {
                        generation,
                        settings,
                        carve,
                        reload,
                        progress,
                    } => {
                        if reload {
                            self.job = None;
                        }
                        self.calculate(generation, *settings, carve, &progress);
                    }
                    Command::Cleanup {
                        generation,
                        request,
                        settings,
                        design,
                    } => self.cleanup(generation, request, &settings, &design),
                }
            }
        }
    }

    fn send(&self, update: Update) {
        let _ = self.updates.send(update);
        (self.notify)();
    }

    /// Returns a job for `settings`, reusing the loaded source when it has not changed.
    fn job_for(&mut self, settings: Settings) -> Result<&Job, Error> {
        let reuse = self
            .job
            .as_ref()
            .is_some_and(|job| !Job::needs_reload(&job.settings, &settings));
        if reuse {
            let job = self.job.as_mut().expect("checked above");
            job.settings = settings;
        } else {
            self.job = None;
            let job = Job::load(settings, &self.fonts)?;
            self.coverage = Arc::new(job.font.coverage());
            self.job = Some(job);
        }
        Ok(self.job.as_ref().expect("assigned above"))
    }

    fn calculate(&mut self, generation: u64, settings: Settings, carve: bool, progress: &Progress) {
        let fraction = self.fraction.clone();
        let notify = self.notify.clone();
        let observed = progress.observed(move |f| {
            let scaled = (f * 1000.0) as u32;
            if fraction.swap(scaled, Ordering::Relaxed) / 10 != scaled / 10 {
                notify();
            }
        });
        let outcome = self.layout_and_carve(generation, settings, carve, &observed);
        let error = match outcome {
            Ok(()) | Err(Error::Canceled) => None,
            Err(e) => Some(e.to_string()),
        };
        self.send(Update::Stopped { generation, error });
    }

    fn layout_and_carve(
        &mut self,
        generation: u64,
        settings: Settings,
        carve: bool,
        progress: &Progress,
    ) -> Result<(), Error> {
        self.job_for(settings)?;
        let coverage = self.coverage.clone();
        let job = self.job.as_ref().expect("loaded by job_for");
        let warnings = job.warnings.clone();
        let design = job.layout()?;
        let wants_carve = carve && job.settings.cut_type == CutType::VCarve;
        let settings = job.settings.clone();
        self.send(Update::Layout {
            generation,
            design: Arc::new(design.clone()),
            warnings,
            coverage,
        });
        if wants_carve {
            let mut sorted = design;
            let path = CarvePath::calculate(&mut sorted, &settings, progress)?;
            self.send(Update::Carve {
                generation,
                design: Arc::new(sorted),
                carve: Arc::new(path),
            });
        }
        Ok(())
    }

    fn cleanup(&mut self, generation: u64, request: u64, settings: &Settings, design: &Design) {
        let paths = CleanupPaths::calculate(design, settings, CleanupBit::Straight)
            .and_then(|straight| {
                let v_bit = CleanupPaths::calculate(design, settings, CleanupBit::VBit)?;
                Ok((Arc::new(straight.points), Arc::new(v_bit.points)))
            })
            .map_err(|e| e.to_string());
        self.send(Update::Cleanup {
            generation,
            request,
            paths,
        });
    }
}
