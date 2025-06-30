// src/smart_pointer.rs

//! A collection of smart pointer and buffer types for the DjVu library.
//!
//! This module replaces the custom C++ `GSmartPointer.h` and `GSmartPointer.cpp`.
//! The functionality of `GP<T>` is perfectly and safely provided by Rust's
//! standard `std::sync::Arc<T>`, and `GPBuffer<T>` is replaced by `std::vec::Vec<T>`.
//!
//! Using standard library types eliminates manual, unsafe memory management and
//! reference counting, leveraging Rust's ownership and safety guarantees.

use std::sync::Arc;

// --- Smart Pointer Replacement ---

/// A thread-safe, reference-counted smart pointer.
///
/// This is a type alias for `std::sync::Arc<T>`. It is the direct replacement
/// for the C++ `GP<T>` smart pointer. It allows multiple owners for a single
/// piece of heap-allocated data. The data is deallocated automatically when
/// the last owner is dropped.
///
/// Unlike the C++ `GP<T>`, `GP<T>` in Rust does not require the inner type `T`
/// to inherit from a special base class like `GPEnabled`. It can wrap any type.
pub type GP<T> = Arc<T>;

// --- Buffer Replacement ---

/// A resizable, heap-allocated buffer.
///
/// This is a type alias for `std::vec::Vec<T>`. It is the direct replacement
/// for the C++ `GPBuffer<T>` class, which was an RAII wrapper for a C-style
/// array. `Vec<T>` is a safe, efficient, and idiomatic way to manage a
/// dynamic array in Rust.
pub type GpBuffer<T> = Vec<T>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    // A sample struct to be managed by the smart pointer.
    // Note that it does NOT need to derive from any special base trait.
    #[derive(Debug, PartialEq)]
    struct MyObject {
        id: i32,
        name: String,
    }

    impl MyObject {
        fn new(id: i32, name: &str) -> Self {
            MyObject {
                id,
                name: name.to_string(),
            }
        }
    }

    #[test]
    fn test_gp_basic_ownership() {
        // `GP<MyObject> p1 = new MyObject(1, "test");`
        let p1: GP<MyObject> = GP::new(MyObject::new(1, "test"));
        assert_eq!(p1.id, 1);
        assert_eq!(Arc::strong_count(&p1), 1);

        // `GP<MyObject> p2 = p1;`
        let p2 = p1.clone(); // In Rust, cloning an Arc bumps the ref count.
        assert_eq!(Arc::strong_count(&p1), 2);
        assert_eq!(p2.name, "test");

        // `GP<MyObject> p3;`
        let p3: GP<MyObject>;

        // `p3 = p2;`
        p3 = Arc::clone(&p2);
        assert_eq!(Arc::strong_count(&p1), 3);

        // When p2 and p3 go out of scope, their ref counts are decremented.
        drop(p2);
        assert_eq!(Arc::strong_count(&p1), 2);
        drop(p3);
        assert_eq!(Arc::strong_count(&p1), 1);
    }

    #[test]
    fn test_gp_thread_safety() {
        let p1: GP<MyObject> = GP::new(MyObject::new(42, "shared"));
        let mut handles = vec![];

        for i in 0..10 {
            let p_clone = p1.clone();
            let handle = thread::spawn(move || {
                // Each thread has its own owner pointer to the same data.
                assert_eq!(p_clone.id, 42);
                if i == 5 {
                    thread::sleep(std::time::Duration::from_millis(50));
                }
            });
            handles.push(handle);
        }

        // The strong count will be 11 (main thread + 10 spawned threads)
        // Note: This can be racy to check, but it demonstrates the principle.
        assert_eq!(Arc::strong_count(&p1) > 1, true);

        for handle in handles {
            handle.join().unwrap();
        }

        // After all threads finish, only the main thread's Arc remains.
        assert_eq!(Arc::strong_count(&p1), 1);
    }

    #[test]
    fn test_gp_buffer_usage() {
        // `GPBuffer<int> buf(ptr, 10);`
        let mut buf: GpBuffer<i32> = vec![0; 10];
        assert_eq!(buf.len(), 10);
        assert_eq!(buf.capacity(), 10);

        // `buf.resize(20);`
        buf.resize(20, 0);
        assert_eq!(buf.len(), 20);

        // `buf.set(0);`
        buf.fill(0);
        assert!(buf.iter().all(|&x| x == 0));
        
        buf[5] = 99;
        assert_eq!(buf[5], 99);

        // `buf.resize(5);`
        buf.truncate(5);
        assert_eq!(buf.len(), 5);
        
        // The buffer is automatically deallocated when `buf` goes out of scope.
    }
}