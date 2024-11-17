use tokio::sync::Mutex;

pub struct BufferPool {
    pool: Mutex<Vec<Vec<u8>>>,
    buffer_size: usize,
}

impl BufferPool {
    pub fn new(buffer_count: usize, buffer_size: usize) -> Self {
        let pool = (0..buffer_count)
            .map(|_| Vec::with_capacity(buffer_size))
            .collect();
        BufferPool {
            pool: Mutex::new(pool),
            buffer_size,
        }
    }

    pub async fn get_buffer(&self) -> Vec<u8> {
        let mut pool = self.pool.lock().await;
        pool.pop()
            .unwrap_or_else(|| Vec::with_capacity(self.buffer_size))
    }

    pub async fn return_buffer(&self, mut buffer: Vec<u8>) {
        if buffer.capacity() > self.buffer_size {
            buffer = Vec::with_capacity(self.buffer_size);
        } else {
            buffer.clear();
        }
        let mut pool = self.pool.lock().await;
        pool.push(buffer);
    }
}
