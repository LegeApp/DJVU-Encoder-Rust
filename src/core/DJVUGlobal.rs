use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::time::Instant;
use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::ptr;

type DeleteCallback = unsafe extern "C" fn(*mut c_void);
type NewCallback = unsafe extern "C" fn(usize) -> *mut c_void;
type FreeCallback = unsafe extern "C" fn(*mut c_void);
type ReallocCallback = unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void;
type MallocCallback = unsafe extern "C" fn(usize) -> *mut c_void;
type CallocCallback = unsafe extern "C" fn(usize, usize) -> *mut c_void;

static DELETE_HANDLER: AtomicPtr<DeleteCallback> = AtomicPtr::new(ptr::null_mut());
static NEW_HANDLER: AtomicPtr<NewCallback> = AtomicPtr::new(ptr::null_mut());
static FREE_HANDLER: AtomicPtr<FreeCallback> = AtomicPtr::new(ptr::null_mut());
static REALLOC_HANDLER: AtomicPtr<ReallocCallback> = AtomicPtr::new(ptr::null_mut());
static MALLOC_HANDLER: AtomicPtr<MallocCallback> = AtomicPtr::new(ptr::null_mut());
static CALLOC_HANDLER: AtomicPtr<CallocCallback> = AtomicPtr::new(ptr::null_mut());

pub fn set_memory_callbacks(
    free_handler: Option<FreeCallback>,
    realloc_handler: Option<ReallocCallback>,
    malloc_handler: Option<MallocCallback>,
    calloc_handler: Option<CallocCallback>,
) -> bool {
    if let (Some(free_h), Some(realloc_h), Some(malloc_h)) = (free_handler, realloc_handler, malloc_handler) {
        FREE_HANDLER.store(free_h as *mut _, Ordering::SeqCst);
        REALLOC_HANDLER.store(realloc_h as *mut _, Ordering::SeqCst);
        MALLOC_HANDLER.store(malloc_h as *mut _, Ordering::SeqCst);
        if let Some(calloc_h) = calloc_handler {
            CALLOC_HANDLER.store(calloc_h as *mut _, Ordering::SeqCst);
        } else {
            CALLOC_HANDLER.store(ptr::null_mut(), Ordering::SeqCst);
        }
        true
    } else {
        FREE_HANDLER.store(ptr::null_mut(), Ordering::SeqCst);
        REALLOC_HANDLER.store(ptr::null_mut(), Ordering::SeqCst);
        MALLOC_HANDLER.store(ptr::null_mut(), Ordering::SeqCst);
        CALLOC_HANDLER.store(ptr::null_mut(), Ordering::SeqCst);
        false
    }
}

pub fn memory_object_callback(
    delete_handler: Option<DeleteCallback>,
    new_handler: Option<NewCallback>,
) -> bool {
    if let (Some(delete_h), Some(new_h)) = (delete_handler, new_handler) {
        DELETE_HANDLER.store(delete_h as *mut _, Ordering::SeqCst);
        NEW_HANDLER.store(new_h as *mut _, Ordering::SeqCst);
        true
    } else {
        DELETE_HANDLER.store(ptr::null_mut(), Ordering::SeqCst);
        NEW_HANDLER.store(ptr::null_mut(), Ordering::SeqCst);
        false
    }
}

pub fn djvu_new(size: usize) -> *mut c_void {
    if let Some(new_h) = unsafe { NEW_HANDLER.load(Ordering::SeqCst).as_ref() } {
        unsafe { new_h(size.max(1)) }
    } else {
        unsafe { libc::malloc(size.max(1)) as *mut c_void }
    }
}

pub fn djvu_delete(ptr: *mut c_void) {
    if !ptr.is_null() {
        if let Some(delete_h) = unsafe { DELETE_HANDLER.load(Ordering::SeqCst).as_ref() } {
            unsafe { delete_h(ptr) };
        } else {
            unsafe { libc::free(ptr) };
        }
    }
}

pub fn djvu_malloc(size: usize) -> *mut c_void {
    if let Some(malloc_h) = unsafe { MALLOC_HANDLER.load(Ordering::SeqCst).as_ref() } {
        unsafe { malloc_h(size.max(1)) }
    } else {
        unsafe { libc::malloc(size.max(1)) as *mut c_void }
    }
}

pub fn djvu_calloc(size: usize, items: usize) -> *mut c_void {
    let size = size.max(1);
    let items = items.max(1);
    if let Some(calloc_h) = unsafe { CALLOC_HANDLER.load(Ordering::SeqCst).as_ref() } {
        unsafe { calloc_h(size, items) }
    } else if let Some(malloc_h) = unsafe { MALLOC_HANDLER.load(Ordering::SeqCst).as_ref() } {
        let ptr = unsafe { malloc_h(size * items) };
        if !ptr.is_null() && size > 0 && items > 0 {
            unsafe { ptr::write_bytes(ptr, 0, size * items) };
        }
        ptr
    } else {
        unsafe { libc::calloc(size, items) as *mut c_void }
    }
}

pub fn djvu_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    if let Some(realloc_h) = unsafe { REALLOC_HANDLER.load(Ordering::SeqCst).as_ref() } {
        unsafe { realloc_h(ptr, size.max(1)) }
    } else {
        unsafe { libc::realloc(ptr, size.max(1)) as *mut c_void }
    }
}

pub fn djvu_free(ptr: *mut c_void) {
    if !ptr.is_null() {
        if let Some(free_h) = unsafe { FREE_HANDLER.load(Ordering::SeqCst).as_ref() } {
            unsafe { free_h(ptr) };
        } else {
            unsafe { libc::free(ptr) };
        }
    }
}

type ProgressCallback = unsafe extern "C" fn(*const c_char, u64, u64) -> bool;

static PROGRESS_CALLBACK: AtomicPtr<ProgressCallback> = AtomicPtr::new(ptr::null_mut());

pub fn set_progress_callback(callback: Option<ProgressCallback>) -> Option<ProgressCallback> {
    let old = PROGRESS_CALLBACK.swap(callback.map_or(ptr::null_mut(), |c| c as *mut _), Ordering::SeqCst);
    if old.is_null() {
        None
    } else {
        Some(unsafe { *old })
    }
}

pub fn supports_progress_callback() -> bool {
    true
}

pub struct DjVuProgressTask {
    task: String,
    nsteps: u32,
    runtostep: u32,
    startdate: Instant,
    parent: Option<Arc<DjVuProgressTask>>,
}

impl DjVuProgressTask {
    pub fn new(task: &str, nsteps: u32) -> Self {
        DjVuProgressTask {
            task: task.to_string(),
            nsteps,
            runtostep: 0,
            startdate: Instant::now(),
            parent: None,
        }
    }

    pub fn run(&mut self, tostep: u32) {
        if tostep > self.runtostep {
            let curdate = self.startdate.elapsed().as_millis() as u64;
            if let Some(callback) = unsafe { PROGRESS_CALLBACK.load(Ordering::SeqCst).as_ref() } {
                let estdate = self.estimate_enddate(curdate);
                if unsafe { callback(self.task.as_ptr() as *const c_char, curdate, estdate) } {
                    panic!("INTERRUPT");
                }
            }
            self.runtostep = tostep;
        }
    }

    fn estimate_enddate(&self, curdate: u64) -> u64 {
        if self.runtostep > 0 {
            let inprogress = self.runtostep.min(self.nsteps);
            let enddate = curdate + (curdate * self.nsteps as u64) / inprogress as u64;
            enddate
        } else {
            curdate
        }
    }
}

pub fn djvu_print_error_utf8(fmt: &str, args: &[&str]) {
    eprintln!("{}", fmt);
}

pub fn djvu_print_message_utf8(fmt: &str, args: &[&str]) {
    println!("{}", fmt);
}

pub fn djvu_format_error_utf8(fmt: &str, args: &[&str]) {
    eprintln!("{}", fmt);
}

pub fn djvu_write_error(message: &str) {
    eprintln!("{}", message);
}

pub fn djvu_write_message(message: &str) {
    println!("{}", message);
}

pub fn djvu_message_lookup_utf8(msg_buffer: &mut [u8], message: &str) {
    let translated = message.to_string();
    let len = translated.len().min(msg_buffer.len() - 1);
    msg_buffer[..len].copy_from_slice(&translated.as_bytes()[..len]);
    msg_buffer[len] = 0;
}

pub fn djvu_programname(programname: &str) -> &str {
    programname
}