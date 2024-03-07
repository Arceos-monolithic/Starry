//! 实现与futex相关的系统调用
use alloc::collections::{BTreeMap, VecDeque};
use axhal::mem::VirtAddr;
use axsync::Mutex;
use axtask::{AxTaskRef, WaitQueue};

extern crate alloc;

pub static WAIT_FOR_FUTEX: WaitQueue = WaitQueue::new();

pub struct FutexRobustList {
    pub head: usize,
    pub len: usize,
}

impl Default for FutexRobustList {
    fn default() -> Self {
        Self { head: 0, len: 0 }
    }
}
impl FutexRobustList {
    pub fn new(head: usize, len: usize) -> Self {
        Self { head, len }
    }
}

