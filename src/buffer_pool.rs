use std::sync::Arc;
use tokio::sync::Mutex;

pub type SmartVector = Arc<Mutex<Vec<u8>>>;
pub struct BufferPool {
    pool: Arc<Mutex<Vec<SmartVector>>>,
}

impl BufferPool {
    pub fn new(buffer_count: usize, buffer_size: usize) -> Self {
        let pool = (0..buffer_count)
            .map(|_| Arc::new(Mutex::new(Vec::with_capacity(buffer_size))))
            .collect();
        BufferPool {
            pool: Arc::new(Mutex::new(pool)),
        }
    }

    pub async fn get_buffer(&self) -> Option<SmartVector> {
        let mut pool = self.pool.lock().await;
        pool.pop()
    }

    pub async fn return_buffer(&self, buffer: SmartVector) {
        let mut pool = self.pool.lock().await;
        pool.push(buffer);
    }
}
