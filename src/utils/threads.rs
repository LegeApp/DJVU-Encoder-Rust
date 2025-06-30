// src/thread.rs

//! A modern, safe, and portable threading and synchronization toolkit for DjVu encoding.
//!
//! This module replaces the C++ `GThreads` library with idiomatic Rust equivalents
//! from the standard library. This approach provides compile-time safety guarantees
//! and eliminates the need for platform-specific implementations.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// A type alias for a result that may contain a `PoisonError`.
/// A `PoisonError` occurs if a thread panics while holding a lock.
type LockResult<Guard> = Result<Guard, PoisonError<Guard>>;

/// A thread-safe, re-entrant monitor combining a `Mutex` and a `Condvar`.
///
/// This struct is the direct, safe replacement for the C++ `GMonitor`. It provides
/// mutual exclusion and the ability for threads to wait for signals or broadcasts.
///
/// The lock is re-entrant, meaning the same thread can acquire the lock multiple
/// times without deadlocking.
#[derive(Debug)]
pub struct Monitor<T> {
    // We use a tuple to group the state that the Mutex protects.
    // The `T` is the data being protected.
    // The `(thread::ThreadId, u32)` tracks the locking thread and recursion count.
    state: Mutex<(T, (thread::ThreadId, u32))>,
    condvar: Condvar,
}

impl<T> Monitor<T> {
    /// Creates a new `Monitor` protecting the given data.
    pub fn new(data: T) -> Self {
        Monitor {
            state: Mutex::new((data, (thread::current().id(), 0))), // Initially unlocked
            condvar: Condvar::new(),
        }
    }

    /// Acquires a lock on the monitor, blocking the current thread until it is able to do so.
    ///
    /// This method will block if the lock is held by another thread. If the lock is held
    /// by the *current* thread, it will succeed and increment the recursion count.
    ///
    /// Returns a `MutexGuard` which allows access to the protected data. The lock is
    /// released when the guard is dropped.
    pub fn enter(&self) -> LockResult<MonitorGuard<T>> {
        let mut guard = self.state.lock()?;
        let current_thread_id = thread::current().id();

        if guard.1.1 > 0 && guard.1.0 == current_thread_id {
            // Re-entrant lock by the same thread
            guard.1.1 += 1;
        } else {
            // Not locked or locked by another thread, so wait for the lock
            while guard.1.1 > 0 {
                guard = self.condvar.wait(guard)?;
            }
            // We now have the lock
            guard.1.0 = current_thread_id;
            guard.1.1 = 1;
        }

        Ok(guard)
    }

    /// Signals one thread that is waiting on this monitor.
    ///
    /// If there is a waiting thread, it will be woken up.
    /// This is a no-op if no threads are waiting.
    pub fn signal(&self) {
        self.condvar.notify_one();
    }

    /// Wakes up all threads that are waiting on this monitor.
    pub fn broadcast(&self) {
        self.condvar.notify_all();
    }
}

/// A RAII implementation of a scoped lock for a `Monitor`.
///
/// When this structure is created, it will lock the `Monitor`. When it is
/// dropped, the `Monitor` will be unlocked.
pub struct MonitorGuard<'a, T> {
    monitor: &'a Monitor<T>,
}

impl<'a, T> MonitorGuard<'a, T> {
    /// Waits on the monitor's condition variable, atomically unlocking the monitor
    /// and blocking the current thread.
    ///
    /// The thread will be blocked until it is woken up by a `signal` or `broadcast`.
    /// The monitor lock is re-acquired before this method returns.
    pub fn wait(&self) {
        // We can safely unwrap here because this guard proves we hold the lock.
        let mut guard = self.monitor.state.lock().unwrap();
        // The `wait` method atomically unlocks the mutex and waits.
        let _ = self.monitor.condvar.wait(guard).unwrap();
    }

    /// Waits on the monitor's condition variable for a maximum amount of time.
    pub fn wait_timeout(&self, timeout: Duration) {
        let mut guard = self.monitor.state.lock().unwrap();
        let _ = self.monitor.condvar.wait_timeout(guard, timeout).unwrap();
    }

    /// Provides mutable access to the data protected by the `Monitor`.
    pub fn get_mut(&mut self) -> &mut T {
        // Again, unwrap is safe.
        &mut self.monitor.state.lock().unwrap().0
    }
}

// Implement Drop to automatically unlock the monitor.
impl<'a, T> Drop for MonitorGuard<'a, T> {
    fn drop(&mut self) {
        // This unwrap is safe; if we are dropping, we hold the lock.
        let mut guard = self.monitor.state.lock().unwrap();
        guard.1.1 -= 1;
        if guard.1.1 == 0 {
            // Last lock released, notify a waiting thread
            self.monitor.condvar.notify_one();
        }
    }
}

/// A thread-safe, mutable bitflag container.
///
/// This is a direct, safe replacement for the C++ `GSafeFlags`. It uses a `Monitor`
/// to protect the inner flag value, ensuring all operations are atomic.
#[derive(Debug)]
pub struct SafeFlags {
    monitor: Monitor<u32>,
}

impl SafeFlags {
    /// Creates a new `SafeFlags` with an initial value.
    pub fn new(flags: u32) -> Self {
        SafeFlags {
            monitor: Monitor::new(flags),
        }
    }

    /// Gets the current value of the flags.
    pub fn get(&self) -> u32 {
        *self.monitor.enter().unwrap().get_mut()
    }

    /// Sets the flags to a new value and broadcasts to any waiting threads.
    pub fn set(&self, new_flags: u32) {
        let mut guard = self.monitor.enter().unwrap();
        let flags = guard.get_mut();
        if *flags != new_flags {
            *flags = new_flags;
            self.monitor.broadcast();
        }
    }

    /// Atomically tests the flags and modifies them if the test passes.
    ///
    /// This replaces `test_and_modify`.
    ///
    /// # Arguments
    /// * `test_fn` - A closure that takes the current flags and returns `true` if they should be modified.
    /// * `modify_fn` - A closure that takes a mutable reference to the flags and performs the modification.
    ///
    /// Returns `true` if the modification was performed.
    pub fn test_and_modify<F, M>(&self, test_fn: F, modify_fn: M) -> bool
    where
        F: FnOnce(u32) -> bool,
        M: FnOnce(&mut u32),
    {
        let mut guard = self.monitor.enter().unwrap();
        let flags = guard.get_mut();
        if test_fn(*flags) {
            let old_flags = *flags;
            modify_fn(flags);
            if *flags != old_flags {
                self.monitor.broadcast();
            }
            true
        } else {
            false
        }
    }

    /// Atomically waits until a condition is met, then modifies the flags.
    ///
    /// This replaces `wait_and_modify`. The thread will block until `wait_condition`
    /// returns true.
    pub fn wait_and_modify<W, M>(&self, mut wait_condition: W, modify_fn: M)
    where
        W: FnMut(u32) -> bool,
        M: FnOnce(&mut u32),
    {
        let mut guard = self.monitor.enter().unwrap();
        while !wait_condition(*guard.get_mut()) {
            guard.wait();
        }
        
        let old_flags = *guard.get_mut();
        modify_fn(guard.get_mut());
        if *guard.get_mut() != old_flags {
            self.monitor.broadcast();
        }
    }
}

/// A simplified, safe thread spawning function.
///
/// This replaces the `GThread` class. It correctly handles panics in the new thread
/// by catching them and allowing the main thread to see the result.
///
/// Returns a `JoinHandle` which can be used to wait for the thread to complete.
pub fn spawn_thread<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T,
    F: Send + 'static,
    T: Send + 'static,
{
    thread::spawn(move || {
        // The panic hook can be used to log uncaught panics from threads,
        // which is what the original C++ code was trying to do with try/catch.
        // Rust's default behavior is to unwind and report the panic, which is often sufficient.
        f()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn test_monitor_reentrancy() {
        let monitor = Monitor::new(0);
        let guard1 = monitor.enter().unwrap();
        let guard2 = monitor.enter().unwrap(); // Should not deadlock
        assert_eq!(*guard1.monitor.state.lock().unwrap(), (0, (thread::current().id(), 2)));
        drop(guard2);
        assert_eq!(*guard1.monitor.state.lock().unwrap(), (0, (thread::current().id(), 1)));
    }

    #[test]
    fn test_monitor_wait_signal() {
        let monitor = Arc::new(Monitor::new(false));
        let monitor_clone = Arc::clone(&monitor);
        
        let handle = spawn_thread(move || {
            let guard = monitor_clone.enter().unwrap();
            while !*guard.get_mut() {
                guard.wait();
            }
            assert_eq!(*guard.get_mut(), true);
        });

        thread::sleep(Duration::from_millis(10)); // Give the new thread time to wait
        let mut guard = monitor.enter().unwrap();
        *guard.get_mut() = true;
        monitor.signal();
        drop(guard);

        handle.join().unwrap();
    }

    #[test]
    fn test_safe_flags() {
        let flags = Arc::new(SafeFlags::new(0b0001));
        let flags_clone = Arc::clone(&flags);
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);
        
        let handle = spawn_thread(move || {
            flags_clone.wait_and_modify(
                |f| (f & 0b0010) == 0b0010, // Wait until bit 2 is set
                |f| *f |= 0b1000, // Then set bit 4
            );
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        assert_eq!(flags.get(), 0b0001);

        // Modify flags to meet the wait condition
        let modified = flags.test_and_modify(
            |f| (f & 0b0001) == 0b0001,
            |f| *f |= 0b0010,
        );
        assert!(modified);
        assert_eq!(flags.get(), 0b0011);
        
        handle.join().unwrap();
        
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert_eq!(flags.get(), 0b1011);
    }
}