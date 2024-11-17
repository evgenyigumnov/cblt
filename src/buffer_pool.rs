use deadpool::managed;
use deadpool::managed::{Manager, RecycleError, RecycleResult};


pub struct BufferManager {
    pub buffer_size: usize,
}

impl Manager for BufferManager {
    type Type = Vec<u8>;
    type Error = ();

    async fn create(&self) -> Result<Vec<u8>, ()> {
        Ok(Vec::with_capacity(self.buffer_size))
    }

    async fn recycle(&self, obj: &mut Vec<u8>, _: &managed::Metrics) -> RecycleResult<()> {
        if obj.capacity() > self.buffer_size {
            Err(RecycleError::Backend(()))
        } else {
            obj.clear();
            Ok(())
        }
    }
}
pub type Pool = managed::Pool<BufferManager>;
