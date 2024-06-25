//! 支持 futex 相关的 syscall

extern crate alloc;
use alloc::collections::VecDeque;
use axhal::mem::VirtAddr;
use axlog::info;
use axprocess::{
    current_process, current_task,
    futex::{get_futex_key, FutexKey, FutexRobustList, FUTEX_WAIT_TASK, WAIT_FOR_FUTEX},
    yield_now_task,
};
use axtask::CurrentTask;

pub use axstd::time::Instant;
use core::time::Duration;

use crate::{FutexFlags, RobustList, SyscallError, SyscallResult, TimeSecs};

/// Futex requeue操作
///
/// 首先唤醒src_addr对应的futex变量的等待队列中，至多wake_num个任务
///
/// 若原队列中的任务数大于wake_num，则将多余的任务移动到dst_addr对应的futex变量的等待队列中
///
/// 移动的任务数目至多为move_num
///
/// 不考虑检查操作
pub fn futex_requeue(wake_num: u32, move_num: usize, src_addr: VirtAddr, dst_addr: VirtAddr) {
    let key = get_futex_key(src_addr, 0);
    let mut futex_wait_task = FUTEX_WAIT_TASK.lock();
    if !futex_wait_task.contains_key(&key) {
        return;
    }
    let src_wait_task = futex_wait_task.get_mut(&key).unwrap();
    for _ in 0..wake_num {
        if let Some((task, _)) = src_wait_task.pop_front() {
            WAIT_FOR_FUTEX.notify_task(false, &task);
        } else {
            break;
        }
    }
    if !src_wait_task.is_empty() {
        let key_new = get_futex_key(dst_addr, 0);
        let move_num = move_num.min(src_wait_task.len());

        let mut temp_move_task = src_wait_task.drain(..move_num).collect::<VecDeque<_>>();
        let dst_wait_task = futex_wait_task.entry(key_new).or_default();
        dst_wait_task.append(&mut temp_move_task);
    }
}

fn futex_quque(key: FutexKey, curr: &CurrentTask, val: u32) {
    let mut futex_wait_task = FUTEX_WAIT_TASK.lock();
    let wait_list = futex_wait_task.entry(key).or_default();
    wait_list.push_back((curr.as_task_ref().clone(), val));
}

fn futex_unqueue(key: FutexKey, curr: &CurrentTask) -> bool {
    let mut futex_wait_task = FUTEX_WAIT_TASK.lock();
    if futex_wait_task.contains_key(&key) {
        let wait_list = futex_wait_task.get_mut(&key).unwrap();
        if let Some(index) = wait_list
            .iter()
            .position(|(task, _)| task.id() == curr.id())
        {
            wait_list.remove(index);
            return true;
        }
    }
    false
}

fn futex_wait(
    vaddr: VirtAddr,
    futex_op: i32,
    val: u32,
    timeout: usize,
) -> Result<usize, SyscallError> {
    let mut to = false;
    let current_task = current_task();
    let deadline = if timeout != 0 {
        Some(Duration::from_nanos(timeout as u64) + axhal::time::current_time())
    } else {
        None
    };
    loop {
        let key = get_futex_key(vaddr, futex_op);
        let process = current_process();
        if process.manual_alloc_for_lazy(vaddr).is_ok() {
            let real_futex_val = unsafe { (vaddr.as_usize() as *const u32).read_volatile() };
            info!("real val: {:#x}, expected val: {:#x}", real_futex_val, val);
            if real_futex_val != val {
                return Err(SyscallError::EAGAIN);
            }
            futex_quque(key, &current_task, val);

            if let Some(deadline) = deadline {
                let now = axhal::time::current_time();
                to = deadline < now;
            }
            if timeout == 0 || !to {
                yield_now_task();
            }
            // If we were woken (and unqueued), we succeeded, whatever.
            // TODO: plist_del, not just iterate all the list
            if !futex_unqueue(key, &current_task) {
                return Ok(0);
            }
            if to {
                return Err(SyscallError::ETIMEDOUT);
            }
            // we expect signal_pending(current), but we might be the victim
            // of a spurious wakeup as well.
            #[cfg(feature = "signal")]
            if process.have_signals().is_some() {
                // 被信号打断
                return Err(SyscallError::EINTR);
            }
        } else {
            return Err(SyscallError::EFAULT);
        }
    }
}

enum MyResult {
    InstantResult(Instant),
    DurationResult(Duration),
}

fn get_deadline(futex_op: i32, timeout: usize) -> Option<MyResult> {
    const FUTEX_WAIT_BITSET: i32 = 9;
    const FUTEX_CLOCK_REALTIME: i32 = 25;
    match futex_op {
        FUTEX_CLOCK_REALTIME => {
            let now = Instant::now();
            let timeout_duration = Duration::from_nanos(timeout as u64);
            let instant_deadline = now.checked_add(timeout_duration)?;
            Some(MyResult::InstantResult(instant_deadline))
        }
        FUTEX_WAIT_BITSET => {
            let duration_deadline =
                Duration::from_nanos(timeout as u64) + axhal::time::current_time();
            Some(MyResult::DurationResult(duration_deadline))
        }
        _ => None,
    }
}

fn is_timed_out(deadline: &Option<MyResult>) -> bool {
    if let Some(ref deadline) = deadline {
        match deadline {
            MyResult::InstantResult(instant) => {
                return Instant::now().duration_since(*instant) > Duration::from_secs(0);
            }
            MyResult::DurationResult(duration) => {
                return axhal::time::current_time() > *duration;
            }
        }
    }
    false
}

fn futex_wait_bitset(
    vaddr: VirtAddr,
    futex_op: i32,
    val: u32,
    timeout: usize,
    bitset: u32,
) -> Result<usize, SyscallError> {
    let current_task = current_task();
    let deadline = get_deadline(futex_op, timeout);
    loop {
        let key = get_futex_key(vaddr, futex_op);
        let process = current_process();
        if process.manual_alloc_for_lazy(vaddr).is_ok() {
            let real_futex_val = unsafe { (vaddr.as_usize() as *const u32).read_volatile() };
            info!("real val: {:#x}, expected val: {:#x}", real_futex_val, val);
            if real_futex_val != val {
                return Err(SyscallError::EAGAIN);
            }
            futex_quque(key, &current_task, val);

            // if (real_futex_val & bitset) == bitset {
            //     return Ok(0);
            // }
            if timeout == 0 || !is_timed_out(&deadline) {
                yield_now_task();
            }
            // If we were woken (and unqueued), we succeeded, whatever.
            // TODO: plist_del, not just iterate all the list
            if !futex_unqueue(key, &current_task) {
                return Ok(0);
            }
            if is_timed_out(&deadline) {
                return Err(SyscallError::ETIMEDOUT);
            }
            // we expect signal_pending(current), but we might be the victim
            // of a spurious wakeup as well.
            #[cfg(feature = "signal")]
            if process.have_signals().is_some() {
                // 被信号打断
                return Err(SyscallError::EINTR);
            }
        } else {
            return Err(SyscallError::EFAULT);
        }
    }
}

fn futex_wake(vaddr: VirtAddr, futex_op: i32, val: u32) -> Result<usize, SyscallError> {
    let mut ret = 0;
    let key = get_futex_key(vaddr, futex_op);
    // 当前任务释放了锁，所以不需要再次释放
    let mut futex_wait_task = FUTEX_WAIT_TASK.lock();
    if futex_wait_task.contains_key(&key) {
        let wait_list = futex_wait_task.get_mut(&key).unwrap();
        // info!("now task: {}", wait_list.len());
        while let Some((task, _)) = wait_list.pop_front() {
            // 唤醒一个正在等待的任务
            info!("wake task: {}", task.id().as_u64());
            // WAIT_FOR_FUTEX.notify_task(false, &task);
            ret += 1;
            if ret == val {
                break;
            }
        }
    }
    drop(futex_wait_task);
    yield_now_task();
    Ok(ret as usize)
}

fn futex_wake_bitset(
    vaddr: VirtAddr,
    futex_op: i32,
    val: u32,
    bitset: u32,
) -> Result<usize, SyscallError> {
    let mut ret = 0;
    let key = get_futex_key(vaddr, futex_op);
    let mut futex_wait_task = FUTEX_WAIT_TASK.lock();
    if let Some(wait_list) = futex_wait_task.get_mut(&key) {
        for (task, task_bitset) in wait_list.iter() {
            let wakeup = (val == 0 || ret < val) && ((*task_bitset & bitset) != 0);
            if wakeup {
                WAIT_FOR_FUTEX.notify_task(false, &task);
                ret += 1;
            }
            if val != 0 && ret >= val {
                break;
            }
        }

        wait_list.retain(|(_, task_bitset)| (*task_bitset & !bitset) != 0);
        if wait_list.is_empty() {
            futex_wait_task.remove(&key);
        }
    }
    drop(futex_wait_task);
    Ok(ret as usize)
}

/// To do the futex operation
///
/// It may create, remove the futex wait task or requeue the futex wait task
pub fn futex(
    vaddr: VirtAddr,
    futex_op: i32,
    val: u32,
    timeout: usize,
    vaddr2: VirtAddr,
    val2: usize,
    val3: u32,
) -> Result<usize, SyscallError> {
    let flag = FutexFlags::new(futex_op);

    match flag {
        FutexFlags::Wait => futex_wait(vaddr, futex_op, val, timeout),
        FutexFlags::Wake => futex_wake(vaddr, futex_op, val),
        FutexFlags::Requeue => {
            futex_requeue(val, val2, vaddr, vaddr2);
            Ok(0)
        }
        FutexFlags::WaitBitset | FutexFlags::RealTime => {
            futex_wait_bitset(vaddr, futex_op, val, timeout, val3)
        }
        FutexFlags::WakeBitset => futex_wake_bitset(vaddr, futex_op, val, val3),
        _ => {
            Err(SyscallError::EINVAL)
            // return Ok(0);
        }
    }
}

/// # Arguments
/// * vaddr: usize
/// * futex_op: i32
/// * futex_val: u32
/// * time_out_val: usize
/// * vaddr2: usize
/// * val3: u32
pub fn syscall_futex(args: [usize; 6]) -> SyscallResult {
    let vaddr = args[0];
    let futex_op = args[1] as i32;
    let futex_val = args[2] as u32;
    let time_out_val = args[3];
    let vaddr2 = args[4];
    let val3 = args[5] as u32;
    let process = current_process();
    let timeout = if time_out_val != 0 && process.manual_alloc_for_lazy(time_out_val.into()).is_ok()
    {
        let time_sepc: TimeSecs = unsafe { *(time_out_val as *const TimeSecs) };
        time_sepc.turn_to_nanos()
    } else {
        // usize::MAX
        0
    };
    // 释放锁，防止任务无法被调度
    match futex(
        vaddr.into(),
        futex_op,
        futex_val,
        timeout,
        vaddr2.into(),
        time_out_val,
        val3,
    ) {
        Ok(ans) => Ok(ans as isize),
        Err(errno) => Err(errno),
    }
}

/// 内核只发挥存储的作用
/// 但要保证head对应的地址已经被分配
/// # Arguments
/// * head: usize
/// * len: usize
pub fn syscall_set_robust_list(args: [usize; 6]) -> SyscallResult {
    let head = args[0];
    let len = args[1];
    let process = current_process();
    if len != core::mem::size_of::<RobustList>() {
        return Err(SyscallError::EINVAL);
    }
    let curr_id = current_task().id().as_u64();
    if process.manual_alloc_for_lazy(head.into()).is_ok() {
        let mut robust_list = process.robust_list.lock();
        robust_list.insert(curr_id, FutexRobustList::new(head, len));
        Ok(0)
    } else {
        Err(SyscallError::EINVAL)
    }
}

/// 取出对应线程的robust list
/// # Arguments
/// * pid: i32
/// * head: *mut usize
/// * len: *mut usize
pub fn syscall_get_robust_list(args: [usize; 6]) -> SyscallResult {
    let pid = args[0] as i32;
    let head = args[1] as *mut usize;
    let len = args[2] as *mut usize;

    if pid == 0 {
        let process = current_process();
        let curr_id = current_task().id().as_u64();
        if process
            .manual_alloc_for_lazy((head as usize).into())
            .is_ok()
        {
            let robust_list = process.robust_list.lock();
            if robust_list.contains_key(&curr_id) {
                let list = robust_list.get(&curr_id).unwrap();
                unsafe {
                    *head = list.head;
                    *len = list.len;
                }
            } else {
                return Err(SyscallError::EPERM);
            }
            return Ok(0);
        }
        return Err(SyscallError::EPERM);
    }
    Err(SyscallError::EPERM)
}
