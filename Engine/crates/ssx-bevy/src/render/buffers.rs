use std::marker::PhantomData;

use bevy::render::render_resource::encase::internal::WriteInto;
use bevy::render::render_resource::{
    BindingResource, Buffer, BufferDescriptor, BufferUsages, COPY_BUFFER_ALIGNMENT, IntoBinding, ShaderType, encase,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};

pub fn filled_buffer(
    device: &RenderDevice,
    queue: &RenderQueue,
    label: &'static str,
    contents: &[u8],
    usage: BufferUsages,
) -> Buffer {
    let padded = padded_contents(contents);
    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some(label),
        size: padded.len() as u64,
        usage: usage | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    if !padded.is_empty() {
        queue.write_buffer(&buffer, 0, &padded);
    }
    buffer
}

pub fn filled_uniform<T: ShaderType + WriteInto>(
    device: &RenderDevice,
    queue: &RenderQueue,
    label: &'static str,
    value: &T,
) -> Buffer {
    let mut bytes = encase::UniformBuffer::new(Vec::new());
    bytes.write(value).expect("encase writes a uniform into a growing Vec");
    filled_buffer(device, queue, label, bytes.as_ref(), BufferUsages::UNIFORM)
}

pub struct QueuedUniform<T> {
    buffer: Option<Buffer>,
    bytes: Vec<u8>,
    value: PhantomData<fn(&T)>,
}

impl<T> Default for QueuedUniform<T> {
    fn default() -> Self {
        QueuedUniform { buffer: None, bytes: Vec::new(), value: PhantomData }
    }
}

impl<T: ShaderType + WriteInto> QueuedUniform<T> {
    pub fn write(&mut self, device: &RenderDevice, queue: &RenderQueue, label: &'static str, value: &T) {
        let mut encoded = encase::UniformBuffer::new(std::mem::take(&mut self.bytes));
        encoded.write(value).expect("encase writes a uniform into a growing Vec");
        self.bytes = encoded.into_inner();
        let padded = padded_contents(&self.bytes);
        let size = padded.len() as u64;
        let buffer = match &self.buffer {
            Some(b) if b.size() == size => b,
            _ => self.buffer.insert(device.create_buffer(&BufferDescriptor {
                label: Some(label),
                size,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })),
        };
        if !padded.is_empty() {
            queue.write_buffer(buffer, 0, &padded);
        }
    }

    pub fn binding(&self) -> Option<BindingResource<'_>> {
        self.buffer.as_ref().map(|b| b.as_entire_binding())
    }
}

impl<'a, T: ShaderType + WriteInto> IntoBinding<'a> for &'a QueuedUniform<T> {
    fn into_binding(self) -> BindingResource<'a> {
        self.binding().expect("a uniform is written before it is bound")
    }
}

fn padded_contents(contents: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    let size = contents.len().next_multiple_of(COPY_BUFFER_ALIGNMENT as usize);
    if size == contents.len() {
        return std::borrow::Cow::Borrowed(contents);
    }
    let mut padded = contents.to_vec();
    padded.resize(size, 0);
    std::borrow::Cow::Owned(padded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contents_are_padded_to_whole_words_as_create_buffer_init_sizes_them() {
        assert!(padded_contents(&[]).is_empty());
        assert_eq!(&*padded_contents(&[1, 2, 3, 4]), &[1, 2, 3, 4]);
        assert_eq!(&*padded_contents(&[1, 2, 3, 4, 5, 6]), &[1, 2, 3, 4, 5, 6, 0, 0]);
        assert_eq!(&*padded_contents(&[7]), &[7, 0, 0, 0]);
        let indices: Vec<u8> = [1u16, 2, 3].iter().flat_map(|i| i.to_le_bytes()).collect();
        assert_eq!(padded_contents(&indices).len(), 8);
    }
}
