use crate::LinuxResult;
use crate::protocol::{EndpointState, WireStatus};
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::size_of;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};

pub const RING_MAGIC: u32 = 0x434b_5247;
pub const RING_VERSION: u16 = 1;
const LEN_SIZE: usize = size_of::<u32>();

#[repr(C)]
struct RingHeader {
    magic: u32,
    version: u16,
    reserved: u16,
    capacity: u32,
    slot_size: u32,
    producer: AtomicU64,
    consumer: AtomicU64,
}

pub struct SharedRing {
    ptr: NonNull<u8>,
    len: usize,
    _file: File,
}

impl SharedRing {
    pub fn create(path: &Path, capacity: u32, slot_size: u32) -> LinuxResult<Self> {
        let len = total_len(capacity, slot_size)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_len(len as u64)?;
        let ring = Self::map(file, len)?;
        ring.init(capacity, slot_size);
        Ok(ring)
    }

    pub fn open(path: &Path) -> LinuxResult<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let len = file.metadata()?.len() as usize;
        let ring = Self::map(file, len)?;
        ring.validate()?;
        Ok(ring)
    }

    pub fn capacity(&self) -> u32 {
        self.header().capacity
    }

    pub fn slot_size(&self) -> u32 {
        self.header().slot_size
    }

    pub fn send(&self, endpoint: &EndpointState, payload: &[u8]) -> Result<(), WireStatus> {
        match endpoint.status() {
            WireStatus::Ok => {}
            status => return Err(status),
        }
        let header = self.header();
        if payload.len() > header.slot_size as usize {
            return Err(WireStatus::InvalidObject);
        }
        let producer = header.producer.load(Ordering::Relaxed);
        let consumer = header.consumer.load(Ordering::Acquire);
        if producer.wrapping_sub(consumer) >= header.capacity as u64 {
            return Err(WireStatus::QueueFull);
        }

        let slot = producer % header.capacity as u64;
        // SAFETY: slot_ptr is within the mmap range because slot is modulo
        // capacity, slot_size was validated from the ring header, and the file
        // length was computed as header + capacity * stride. The producer is
        // the only writer in MVP-0B's SPSC contract.
        unsafe {
            let slot_ptr = self.slot_ptr(slot);
            (slot_ptr as *mut u32).write(payload.len() as u32);
            std::ptr::copy_nonoverlapping(payload.as_ptr(), slot_ptr.add(LEN_SIZE), payload.len());
        }
        header
            .producer
            .store(producer.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    pub fn recv(&self, endpoint: &EndpointState) -> Result<Option<Vec<u8>>, WireStatus> {
        match endpoint.status() {
            WireStatus::Ok => {}
            status => return Err(status),
        }
        let header = self.header();
        let consumer = header.consumer.load(Ordering::Relaxed);
        let producer = header.producer.load(Ordering::Acquire);
        if consumer == producer {
            return Ok(None);
        }

        let slot = consumer % header.capacity as u64;
        // SAFETY: slot_ptr is within the mmap range by the same bounds argument
        // as send(). The consumer is the only reader that advances consumer in
        // MVP-0B's SPSC contract, and producer Acquire observes the payload
        // writes that happened before producer Release.
        let payload = unsafe {
            let slot_ptr = self.slot_ptr(slot);
            let len = (slot_ptr as *const u32).read() as usize;
            if len > header.slot_size as usize {
                return Err(WireStatus::InvalidObject);
            }
            std::slice::from_raw_parts(slot_ptr.add(LEN_SIZE), len).to_vec()
        };
        header
            .consumer
            .store(consumer.wrapping_add(1), Ordering::Release);
        Ok(Some(payload))
    }

    fn map(file: File, len: usize) -> LinuxResult<Self> {
        if len < size_of::<RingHeader>() {
            return Err("shared ring file is too small".into());
        }
        // SAFETY: mmap is called with a valid file descriptor and the returned
        // pointer is checked against MAP_FAILED before being wrapped. The
        // mapping is unmapped exactly once in Drop.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                std::os::fd::AsRawFd::as_raw_fd(&file),
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error().into());
        }
        let ptr = NonNull::new(ptr.cast::<u8>()).ok_or("mmap returned null")?;
        Ok(Self {
            ptr,
            len,
            _file: file,
        })
    }

    fn init(&self, capacity: u32, slot_size: u32) {
        let header = RingHeader {
            magic: RING_MAGIC,
            version: RING_VERSION,
            reserved: 0,
            capacity,
            slot_size,
            producer: AtomicU64::new(0),
            consumer: AtomicU64::new(0),
        };
        // SAFETY: the mapping is at least the size of RingHeader by construction
        // and is uniquely initialized by the creator before peers open it.
        unsafe {
            (self.ptr.as_ptr() as *mut RingHeader).write(header);
        }
    }

    fn validate(&self) -> LinuxResult<()> {
        let header = self.header();
        if header.magic != RING_MAGIC {
            return Err("bad shared ring magic".into());
        }
        if header.version != RING_VERSION {
            return Err("unsupported shared ring version".into());
        }
        let expected = total_len(header.capacity, header.slot_size)?;
        if expected != self.len {
            return Err("shared ring file length does not match header".into());
        }
        Ok(())
    }

    fn header(&self) -> &RingHeader {
        // SAFETY: SharedRing is only constructed from a valid mmap whose length
        // is at least RingHeader. The header lives for the mapping lifetime.
        unsafe { &*(self.ptr.as_ptr() as *const RingHeader) }
    }

    unsafe fn slot_ptr(&self, slot: u64) -> *mut u8 {
        let header_len = size_of::<RingHeader>();
        let stride = slot_stride(self.header().slot_size);
        // SAFETY: callers ensure slot is modulo capacity and therefore within
        // the mapped slot region.
        unsafe { self.ptr.as_ptr().add(header_len + slot as usize * stride) }
    }
}

impl Drop for SharedRing {
    fn drop(&mut self) {
        // SAFETY: ptr/len come from a successful mmap call and this Drop runs
        // exactly once for the mapping owner.
        unsafe {
            libc::munmap(self.ptr.as_ptr().cast(), self.len);
        }
    }
}

fn total_len(capacity: u32, slot_size: u32) -> LinuxResult<usize> {
    let slots = capacity as usize;
    let stride = slot_stride(slot_size);
    size_of::<RingHeader>()
        .checked_add(
            slots
                .checked_mul(stride)
                .ok_or("shared ring size overflow")?,
        )
        .ok_or_else(|| "shared ring size overflow".into())
}

fn slot_stride(slot_size: u32) -> usize {
    LEN_SIZE + slot_size as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{WireCap, WireRights};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn shared_ring_round_trip() {
        let path = std::env::temp_dir().join(format!(
            "ck-ring-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let producer = SharedRing::create(&path, 4, 128).unwrap();
        let consumer = SharedRing::open(&path).unwrap();
        let endpoint = EndpointState::new(WireCap {
            id: 1,
            rights: WireRights { bits: 0 },
        });

        producer.send(&endpoint, b"packet").unwrap();

        assert_eq!(consumer.recv(&endpoint).unwrap(), Some(b"packet".to_vec()));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn revoked_endpoint_blocks_send() {
        let path = std::env::temp_dir().join(format!(
            "ck-ring-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let ring = SharedRing::create(&path, 4, 128).unwrap();
        let mut endpoint = EndpointState::new(WireCap {
            id: 1,
            rights: WireRights { bits: 0 },
        });
        endpoint.mark_revoked();

        assert_eq!(ring.send(&endpoint, b"packet"), Err(WireStatus::Revoked));
        let _ = std::fs::remove_file(path);
    }
}
