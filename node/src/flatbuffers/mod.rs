
#[allow(dead_code, unused_imports)]
#[path = "./object_generated.rs"]
mod object_generated;
pub use object_generated::object;

use std::ops::Deref;

pub fn build_object(content_type: &str, content: &[u8]) -> OwnedBuffer {
    let mut builder = flatbuffers::FlatBufferBuilder::new_with_capacity(content_type.len() + content.len() + 12);
    let content_type = builder.create_string(content_type);
    let content = builder.create_vector(content);
    
    let object = object::Object::create(&mut builder, &object::ObjectArgs {
        content_type: Some(content_type),
        content: Some(content),
    });

    builder.finish(object, None);
    builder.collapse().into()
}

pub struct OwnedBuffer {
    buffer: Vec<u8>,
    start: usize,
}

impl Deref for OwnedBuffer {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.buffer[self.start..]
    }
}

impl AsRef<[u8]> for OwnedBuffer {
    fn as_ref(&self) -> &[u8] {
        &self.buffer[self.start..]
    }
}

impl From<(Vec<u8>, usize)> for OwnedBuffer {
    fn from((buffer, start): (Vec<u8>, usize)) -> OwnedBuffer {
        OwnedBuffer { buffer, start}
    }
}
