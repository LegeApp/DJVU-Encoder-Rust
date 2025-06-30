// src/array.rs

//! A collection of type aliases and utilities for working with dynamic arrays.
//!
//! This module replaces the C++ `Arrays.h` and `Arrays.cpp` implementation.
//! The complex, hand-rolled, copy-on-write `TArray`, `DArray`, and `DPArray`
//! classes are all replaced by Rust's standard, safe, and efficient `Vec<T>`.
//!
//! The Rust compiler and standard library handle all memory management, generic
//! instantiation, and safety checks automatically, making the custom C++
//! implementation entirely obsolete.

use std::cmp::Ordering;

// --- Type Aliases for Clarity ---

/// A dynamic array for simple, `Copy`-able types (e.g., `i32`, `f64`, `char`).
/// This is the conceptual replacement for the C++ `TArray`.
pub type TArray<T> = Vec<T>;

/// A dynamic array for general types that can be cloned.
/// This is the conceptual replacement for the C++ `DArray`.
pub type DArray<T> = Vec<T>;

/// A dynamic array for storing `Arc<T>` (Atomically Referenced-Counted) smart pointers.
/// This is the conceptual replacement for the C++ `DPArray` and `GP`-based arrays.
/// Using `Arc<T>` allows multiple parts of the code to share ownership of an object.
use std::sync::Arc;
pub type DPArray<T> = Vec<Arc<T>>;

// --- Extension Trait for Custom Sorting (Optional but good practice) ---

/// An extension trait to provide a quicksort implementation similar to the C++ version.
///
/// While `Vec::sort` (which uses a merge sort) is generally excellent, this trait
/// demonstrates how to add the specific quicksort logic from the original C++ code
/// if its performance characteristics (e.g., for partially sorted data) were critical.
pub trait DjvuVecExt<T> {
    /// Sorts a slice in-place using a recursive quicksort algorithm.
    fn djvu_qsort_by<F>(&mut self, compare: F)
    where
        F: Fn(&T, &T) -> Ordering;
}

impl<T: Clone> DjvuVecExt<T> for [T] {
    fn djvu_qsort_by<F>(&mut self, compare: F)
    where
        F: Fn(&T, &T) -> Ordering,
    {
        // Use Rust's standard unstable sort, which is an introsort (a hybrid
        // quicksort) and is highly optimized. It's almost always a better
        // choice than a hand-rolled quicksort. We can provide the same API
        // while using a superior underlying implementation.
        self.sort_unstable_by(compare);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tarray_usage() {
        // The C++ `TArray<int> a(0, 4)` becomes:
        let mut a: TArray<i32> = vec![0; 5]; // A vec of 5 integers, all initialized to 0.
        
        // `a[2] = 100;`
        a[2] = 100;
        assert_eq!(a[2], 100);

        // `a.resize(10)` becomes:
        a.resize(10, 0); // Resize to 10 elements, filling with 0.
        assert_eq!(a.len(), 10);
        
        // `a.del(2)` becomes:
        a.remove(2);
        assert_eq!(a.len(), 9);
        assert_eq!(a[2], 0); // The element at index 3 moved to index 2.

        // `a.ins(5, 99)` becomes:
        a.insert(5, 99);
        assert_eq!(a[5], 99);
        assert_eq!(a.len(), 10);
    }

    #[test]
    fn test_darray_usage() {
        // `DArray<GString> arr;`
        let mut arr: DArray<String> = DArray::new();
        
        // `arr.ins(0, "hello");`
        arr.insert(0, "hello".to_string());
        arr.insert(1, "world".to_string());
        assert_eq!(arr, &["hello", "world"]);

        // `DArray<GString> arr2 = arr;` (in C++ this is a cheap copy-on-write)
        // In Rust, this is an explicit clone, which is clearer about the cost.
        let mut arr2 = arr.clone();
        
        // `arr2[0] = "goodbye";`
        arr2[0] = "goodbye".to_string();

        // The original array `arr` is unaffected.
        assert_eq!(arr, &["hello", "world"]);
        assert_eq!(arr2, &["goodbye", "world"]);
    }
    
    #[test]
    fn test_dparray_usage() {
        #[derive(Debug, PartialEq)]
        struct MyObject {
            id: i32,
        }

        // `DPArray<MyObject> arr;`
        // `arr.ins(0, new MyObject(1));`
        let mut arr: DPArray<MyObject> = DPArray::new();
        arr.insert(0, Arc::new(MyObject { id: 1 }));
        arr.insert(1, Arc::new(MyObject { id: 2 }));
        
        // `DPArray<MyObject> arr2 = arr;`
        // In Rust, we clone the `Vec`, which clones the `Arc`s.
        // This is cheap and achieves the shared ownership goal of DPArray.
        let arr2 = arr.clone();

        // Both `arr` and `arr2` now point to the same `MyObject` instances.
        assert!(Arc::ptr_eq(&arr[0], &arr2[0]));
        assert_eq!(arr[0].id, 1);
        assert_eq!(arr2[0].id, 1);
    }
    
    #[test]
    fn test_djvu_qsort() {
        let mut numbers: TArray<i32> = vec![5, 1, 4, 2, 8, 0];
        numbers.djvu_qsort_by(|a, b| a.cmp(b));
        assert_eq!(numbers, vec![0, 1, 2, 4, 5, 8]);
    }

    #[test]
    fn bytestream_get_data_hack_replacement() {
        // The `ByteStream::get_data` hack from Arrays.cpp can be
        // replaced by standard `Read` trait methods.
        use std::io::{Cursor, Read};
        
        let mut stream = Cursor::new(vec![10, 20, 30, 40]);
        
        // `get_data` reads the whole stream into a TArray.
        // The Rust equivalent is `read_to_end`.
        let mut data: TArray<u8> = TArray::new();
        let bytes_read = stream.read_to_end(&mut data).unwrap();
        
        assert_eq!(bytes_read, 4);
        assert_eq!(data, vec![10, 20, 30, 40]);
    }
}