use std::{
    ops::{Deref, Index},
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
};

#[derive(Default)]
pub struct TwccTime(AtomicI64);

impl TwccTime {
    pub fn store(&self, time: i64) {
        self.0.store(time, Ordering::Release);
    }

    pub fn load(&self) -> i64 {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone)]
#[repr(transparent)]
pub struct TwccDataMap(Arc<TwccDataMapShared>);

impl Deref for TwccDataMap {
    type Target = Arc<TwccDataMapShared>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl TwccDataMap {
    pub fn new() -> Self {
        TwccDataMap(Arc::new(TwccDataMapShared::new()))
    }
}

/// A helper `Vec` that is restricted to only indexing operations.
pub struct TwccDataMapShared(Vec<TwccTime>);

impl Index<u16> for TwccDataMapShared {
    type Output = TwccTime;

    fn index(&self, index: u16) -> &Self::Output {
        // SAFETY: The inner vec was allocated to hold exactly u16::MAX values
        unsafe { self.0.get_unchecked(index as usize) }
    }
}

impl TwccDataMapShared {
    fn new() -> Self {
        const ALLOC_SIZE: usize = u16::MAX as usize;
        let mut vec = Vec::new();
        vec.reserve_exact(ALLOC_SIZE);
        for _ in 0..ALLOC_SIZE {
            vec.push(TwccTime::default())
        }
        Self(vec)
    }
}
