use bevy::prelude::*;
use bevy::render::render_resource::{Buffer, BufferDescriptor, BufferUsages, CommandEncoder, CommandEncoderDescriptor};
use bevy::render::renderer::{RenderDevice, RenderQueue};

#[derive(Resource, Default)]
pub struct HdrAdaptation {
    quarter: UVec2,
    level0: Option<Buffer>,
    initialised: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdaptMode {
    Adapt,
    Current,
    Clear,
    Black,
}

impl AdaptMode {
    pub fn code(self) -> f32 {
        match self {
            AdaptMode::Adapt => 0.0,
            AdaptMode::Current => 1.0,
            AdaptMode::Clear => 2.0,
            AdaptMode::Black => 3.0,
        }
    }
}

impl HdrAdaptation {
    pub fn begin(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        q: UVec2,
        adapted: &Buffer,
        fresh: bool,
    ) -> AdaptMode {
        if self.level0.is_none() || self.quarter != q {
            self.level0 = Some(device.create_buffer(&BufferDescriptor {
                label: Some("ssx_hdr_adapt_level0"),
                size: Self::bytes(q),
                usage: BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.quarter = q;
            self.initialised = false;
        }
        if fresh
            && self.initialised
            && let Some(l0) = &self.level0
        {
            let mut encoder =
                device.create_command_encoder(&CommandEncoderDescriptor { label: Some("ssx_hdr_adapt_seed") });
            encoder.copy_buffer_to_buffer(l0, 0, adapted, 0, Self::bytes(q));
            queue.submit([encoder.finish()]);
        }
        if self.initialised { AdaptMode::Adapt } else { AdaptMode::Clear }
    }

    pub fn ran(&mut self) {
        self.initialised = true;
    }

    pub fn store(&self, encoder: &mut CommandEncoder, q: UVec2, adapted: &Buffer) {
        if let Some(l0) = self.level0.as_ref().filter(|_| self.quarter == q) {
            encoder.copy_buffer_to_buffer(adapted, 0, l0, 0, Self::bytes(q));
        }
    }

    fn bytes(q: UVec2) -> u64 {
        u64::from(q.x.max(1) * q.y.max(1)) * 4
    }
}
