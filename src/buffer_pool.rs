use std::sync::Arc;
use tokio::sync::Mutex;

pub type SmartVector = Arc<Mutex<Vec<u8>>>;
pub struct BufferPool {
    pool: Arc<Mutex<Vec<SmartVector>>>,
    buffer_size: usize,
}

impl BufferPool {
    pub fn new(buffer_count: usize, buffer_size: usize) -> Self {
        let pool = (0..buffer_count)
            .map(|_| Arc::new(Mutex::new(Vec::with_capacity(buffer_size))))
            .collect();
        BufferPool {
            pool: Arc::new(Mutex::new(pool)),
            buffer_size,
        }
    }

    pub async fn get_buffer(&self) -> SmartVector {
        let mut pool = self.pool.lock().await;
        if let Some(buffer) = pool.pop() {
            buffer
        } else {
            Arc::new(Mutex::new(Vec::with_capacity(self.buffer_size)))
        }
    }

    pub async fn return_buffer(&self, buffer: SmartVector) {
        let mut pool = self.pool.lock().await;
        let buff = buffer.lock().await;

        if buff.capacity() > self.buffer_size {
            let new_buffer = Arc::new(Mutex::new(Vec::with_capacity(self.buffer_size)));
            pool.push(new_buffer)
        } else {
            drop(buff);
            pool.push(buffer)
        }
    }
}
