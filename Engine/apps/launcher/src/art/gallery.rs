use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions, Vec2};

use super::{Request, Slot, Source, loader};

pub struct Art {
    pub texture: TextureHandle,
    pub size: Vec2,
    pub source: Source,
    pub note: String,
}

enum Message {
    Picture { slot: Slot, source: Source, note: String, image: ColorImage },
    Done(Vec<String>),
}

type Waker = Arc<Mutex<Option<egui::Context>>>;

pub struct Gallery {
    arts: [Option<Art>; 6],
    incoming: Receiver<Message>,
    waker: Waker,
    pub problems: Vec<String>,
    pub loading: bool,
}

fn spawn(request: Request, waker: Waker) -> Receiver<Message> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let wake = || {
            if let Ok(ctx) = waker.lock()
                && let Some(ctx) = ctx.as_ref()
            {
                ctx.request_repaint();
            }
        };
        let problems = loader::load(&request, &mut |l| {
            let image = l.picture.to_egui();
            let _ = tx.send(Message::Picture { slot: l.slot, source: l.source, note: l.note, image });
            wake();
        });
        let _ = tx.send(Message::Done(problems));
        wake();
    });
    rx
}

impl Gallery {
    pub fn start(request: Request) -> Self {
        let waker = Waker::default();
        let incoming = spawn(request, waker.clone());
        Self { arts: Default::default(), incoming, waker, problems: Vec::new(), loading: true }
    }

    pub fn restart(&mut self, request: Request) {
        self.incoming = spawn(request, self.waker.clone());
        self.loading = true;
    }

    pub fn attach(&self, ctx: &egui::Context) {
        if let Ok(mut waker) = self.waker.lock() {
            *waker = Some(ctx.clone());
        }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.incoming.try_recv() {
            match message {
                Message::Picture { slot, source, note, image } => {
                    let size = Vec2::new(image.size[0] as f32, image.size[1] as f32);
                    let texture = ctx.load_texture(format!("art-{}", slot.name()), image, TextureOptions::LINEAR);
                    self.arts[slot.index()] = Some(Art { texture, size, source, note });
                }
                Message::Done(problems) => {
                    self.problems = problems;
                    self.loading = false;
                }
            }
        }
    }

    pub fn ready(&self) -> usize {
        self.arts.iter().flatten().count()
    }

    pub fn get(&self, slot: Slot) -> Option<&Art> {
        self.arts[slot.index()].as_ref()
    }
}
