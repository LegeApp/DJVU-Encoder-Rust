use std::sync::{Arc, Mutex, Weak};
use std::thread::{self, JoinHandle};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

bitflags::bitflags! {
    pub struct Flags: u32 {
        const DECODING = 1;
        const DECODE_OK = 2;
        const DECODE_FAILED = 4;
        const DECODE_STOPPED = 8;
        const DATA_PRESENT = 16;
        const ALL_DATA_PRESENT = 32;
        const INCL_FILES_CREATED = 64;
        const MODIFIED = 128;
        const DONT_START_DECODE = 256;
        const STOPPED = 512;
        const BLOCKED_STOPPED = 1024;
        const CAN_COMPRESS = 2048;
        const NEEDS_COMPRESSION = 4096;
    }
}

#[derive(Debug)]
pub enum DjVuError {
    DecodingError(String),
    IoError(std::io::Error),
}

type Result<T> = std::result::Result<T, DjVuError>;

pub struct DjVuFile {
    url: Url,
    data_pool: DataPool,
    info: Mutex<Option<DjVuInfo>>,
    bg44: Mutex<Option<IW44Image>>,
    bgpm: Mutex<Option<GPixmap>>,
    fgjb: Mutex<Option<JB2Image>>,
    fgjd: Mutex<Option<JB2Dict>>,
    fgpm: Mutex<Option<GPixmap>>,
    fgbc: Mutex<Option<DjVuPalette>>,
    anno: Mutex<Option<ByteStream>>,
    text: Mutex<Option<ByteStream>>,
    meta: Mutex<Option<ByteStream>>,
    dir: Mutex<Option<DjVuNavDir>>,
    description: Mutex<String>,
    mimetype: Mutex<String>,
    file_size: Mutex<u32>,
    flags: Mutex<Flags>,
    inc_files: Mutex<Vec<Arc<DjVuFile>>>,
    decode_thread: Mutex<Option<JoinHandle<()>>>,
    stop_flag: Arc<AtomicBool>,
    recover_errors: ErrorRecoveryAction,
    verbose_eof: bool,
    chunks_number: Mutex<i32>,
}

impl DjVuFile {
    pub fn create_from_stream(stream: ByteStream) -> Arc<Self> {
        let file = DjVuFile {
            url: Url::from_string(format!("djvufile:/{:p}.djvu", &stream)),
            data_pool: DataPool::new(stream),
            info: Mutex::new(None),
            bg44: Mutex::new(None),
            bgpm: Mutex::new(None),
            fgjb: Mutex::new(None),
            fgjd: Mutex::new(None),
            fgpm: Mutex::new(None),
            fgbc: Mutex::new(None),
            anno: Mutex::new(None),
            text: Mutex::new(None),
            meta: Mutex::new(None),
            dir: Mutex::new(None),
            description: Mutex::new(String::new()),
            mimetype: Mutex::new(String::new()),
            file_size: Mutex::new(0),
            flags: Mutex::new(Flags::empty()),
            inc_files: Mutex::new(Vec::new()),
            decode_thread: Mutex::new(None),
            stop_flag: Arc::new(AtomicBool::new(false)),
            recover_errors: ErrorRecoveryAction::Abort,
            verbose_eof: true,
            chunks_number: Mutex::new(-1),
        };
        let arc_file = Arc::new(file);
        arc_file.data_pool.add_trigger(-1, |file| file.trigger_cb());
        arc_file
    }

    pub fn create_from_url(url: Url, port: Option<DjVuPort>) -> Arc<Self> {
        let pcaster = DjVuPortcaster::global();
        let file = DjVuFile {
            url,
            data_pool: pcaster.request_data(&url).unwrap_or_else(|| DataPool::empty()),
            info: Mutex::new(None),
            bg44: Mutex::new(None),
            bgpm: Mutex::new(None),
            fgjb: Mutex::new(None),
            fgjd: Mutex::new(None),
            fgpm: Mutex::new(None),
            fgbc: Mutex::new(None),
            anno: Mutex::new(None),
            text: Mutex::new(None),
            meta: Mutex::new(None),
            dir: Mutex::new(None),
            description: Mutex::new(String::new()),
            mimetype: Mutex::new(String::new()),
            file_size: Mutex::new(0),
            flags: Mutex::new(Flags::empty()),
            inc_files: Mutex::new(Vec::new()),
            decode_thread: Mutex::new(None),
            stop_flag: Arc::new(AtomicBool::new(false)),
            recover_errors: ErrorRecoveryAction::Abort,
            verbose_eof: true,
            chunks_number: Mutex::new(-1),
        };
        let arc_file = Arc::new(file);
        pcaster.add_route(&arc_file, port.unwrap_or_else(|| DjVuSimplePort::new()));
        arc_file.data_pool.add_trigger(-1, |file| file.trigger_cb());
        arc_file
    }

    pub fn start_decode(&self) {
        let mut flags = self.flags.lock().unwrap();
        if !flags.contains(Flags::DONT_START_DECODE) && !flags.contains(Flags::DECODING) {
            if flags.contains(Flags::DECODE_STOPPED) {
                self.reset();
            }
            flags.remove(Flags::DECODE_OK | Flags::DECODE_STOPPED | Flags::DECODE_FAILED);
            flags.insert(Flags::DECODING);
            let arc_self = Arc::clone(&self.arc_self());
            let stop_flag = Arc::clone(&self.stop_flag);
            let handle = thread::spawn(move || {
                while !stop_flag.load(Ordering::SeqCst) {
                    if let Err(e) = arc_self.decode_func() {
                        arc_self.handle_decode_error(e);
                    }
                }
            });
            *self.decode_thread.lock().unwrap() = Some(handle);
        }
    }

    pub fn stop_decode(&self, sync: bool) {
        self.stop_flag.store(true, Ordering::SeqCst);
        let inc_files = self.inc_files.lock().unwrap();
        for file in inc_files.iter() {
            file.stop_decode(false);
        }
        if sync {
            if let Some(handle) = self.decode_thread.lock().unwrap().take() {
                handle.join().unwrap();
            }
        }
    }

    pub fn decode_func(&self) -> Result<()> {
        let bs = self.data_pool.get_stream()?;
        let mut iff = IFFByteStream::new(bs);
        let chkid = iff.get_chunk()?;
        let (djvi, djvu, iw44) = match chkid.as_str() {
            "FORM:DJVI" => (true, false, false),
            "FORM:DJVU" => (false, true, false),
            "FORM:PM44" | "FORM:BM44" => (false, false, true),
            _ => return Err(DjVuError::DecodingError("Unexpected image format".into())),
        };
        *self.mimetype.lock().unwrap() = if djvi || djvu { "image/x.djvu" } else { "image/x-iw44" }.into();
        while let Some((chkid, chunk_bs)) = iff.get_chunk() {
            self.decode_chunk(&chkid, chunk_bs, djvi, djvu, iw44)?;
        }
        Ok(())
    }

    fn decode_chunk(&self, chkid: &str, bs: ByteStream, djvi: bool, djvu: bool, iw44: bool) -> Result<()> {
        match chkid {
            "INFO" if djvu || djvi => {
                let info = DjVuInfo::decode(bs)?;
                *self.info.lock().unwrap() = Some(info);
            }
            "INCL" if djvi || djvu || iw44 => {
                if let Some(file) = self.process_incl_chunk(bs) {
                    file.start_decode();
                }
            }
            "Djbz" if djvu || djvi => {
                let fgjd = JB2Dict::decode(bs)?;
                *self.fgjd.lock().unwrap() = Some(fgjd);
            }
            "Sjbz" if djvu || djvi => {
                let fgjb = JB2Image::decode(bs, self.get_fgjd)?;
                *self.fgjb.lock().unwrap() = Some(fgjb);
            }
            "BG44" if djvu || djvi => {
                let mut bg44 = self.bg44.lock().unwrap();
                if bg44.is_none() {
                    *bg44 = Some(IW44Image::decode_chunk(bs, IW44ImageType::Color)?);
                } else {
                    bg44.as_mut().unwrap().refine(bs)?;
                }
            }
            "FGbz" if djvu || djvi => {
                let fgbc = DjVuPalette::decode(bs)?;
                *self.fgbc.lock().unwrap() = Some(fgbc);
            }
            "NDIR" => {
                let dir = DjVuNavDir::decode(bs, &self.url)?;
                *self.dir.lock().unwrap() = Some(dir);
            }
            _ if is_annotation(chkid) => {
                let mut anno = self.anno.lock().unwrap();
                if anno.is_none() {
                    *anno = Some(ByteStream::new());
                }
                let mut a = anno.as_mut().unwrap();
                if a.position() > 0 {
                    a.write_u8(0)?;
                }
                a.copy_from(bs)?;
            }
            _ if is_text(chkid) => {
                let mut text = self.text.lock().unwrap();
                if text.is_none() {
                    *text = Some(ByteStream::new());
                }
                let mut t = text.as_mut().unwrap();
                if t.position() > 0 {
                    t.write_u8(0)?;
                }
                t.copy_from(bs)?;
            }
            _ if is_meta(chkid) => {
                let mut meta = self.meta.lock().unwrap();
                if meta.is_none() {
                    *meta = Some(ByteStream::new());
                }
                let mut m = meta.as_mut().unwrap();
                if m.position() > 0 {
                    m.write_u8(0)?;
                }
                m.copy_from(bs)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn process_incl_chunk(&self, mut bs: ByteStream) -> Option<Arc<DjVuFile>> {
        let incl_str = bs.read_string()?;
        let url = DjVuPortcaster::global().id_to_url(&self.url, &incl_str)?;
        let mut inc_files = self.inc_files.lock().unwrap();
        if let Some(file) = inc_files.iter().find(|f| f.url == url) {
            return Some(Arc::clone(file));
        }
        let file = DjVuPortcaster::global().id_to_file(&self.url, &incl_str)?;
        inc_files.push(Arc::clone(&file));
        Some(file)
    }

    pub fn get_included_files(&self, only_created: bool) -> Vec<Arc<DjVuFile>> {
        if !only_created && !self.are_incl_files_created() {
            self.process_incl_chunks();
        }
        self.inc_files.lock().unwrap().clone()
    }

    pub fn get_merged_anno(&self) -> Option<ByteStream> {
        let mut visited = HashSet::new();
        let mut str_out = ByteStream::new();
        self.get_merged_anno_recursive(&mut str_out, &[], 0, &mut 0, &mut visited);
        if str_out.position() > 0 {
            str_out.seek(0)?;
            Some(str_out)
        } else {
            None
        }
    }

    fn get_merged_anno_recursive(&self, str_out: &mut ByteStream, ignore_list: &[Url], level: i32, max_level: &mut i32, visited: &mut HashSet<Url>) {
        if visited.contains(&self.url) {
            return;
        }
        visited.insert(self.url.clone());
        for file in self.get_included_files(true) {
            file.get_merged_anno_recursive(str_out, ignore_list, level + 1, max_level, visited);
        }
        if !ignore_list.contains(&self.url) {
            if let Some(anno) = self.get_anno() {
                if str_out.position() > 0 {
                    str_out.write_u8(0)?;
                }
                anno.seek(0)?;
                str_out.copy_from(anno)?;
                if level > *max_level {
                    *max_level = level;
                }
            }
        }
    }

    pub fn get_anno(&self) -> Option<ByteStream> {
        let mut str_out = ByteStream::new();
        if let Some(anno) = self.anno.lock().unwrap().as_ref() {
            if str_out.position() > 0 {
                str_out.write_u8(0)?;
            }
            anno.seek(0)?;
            str_out.copy_from(anno)?;
        } else if self.is_data_present() {
            let mut bs = self.data_pool.get_stream()?;
            let mut iff = IFFByteStream::new(bs);
            while let Some((chkid, chunk_bs)) = iff.get_chunk() {
                if is_annotation(&chkid) {
                    if str_out.position() > 0 {
                        str_out.write_u8(0)?;
                    }
                    str_out.copy_from(chunk_bs)?;
                }
            }
        }
        if str_out.position() > 0 {
            str_out.seek(0)?;
            Some(str_out)
        } else {
            None
        }
    }

    pub fn remove_anno(&self) {
        let mut bs = self.data_pool.get_stream()?;
        let mut iff_in = IFFByteStream::new(bs);
        let mut str_out = ByteStream::new();
        let mut iff_out = IFFByteStream::new(&mut str_out);
        let chkid = iff_in.get_chunk()?.unwrap();
        iff_out.put_chunk(&chkid);
        while let Some((chkid, chunk_bs)) = iff_in.get_chunk() {
            if !is_annotation(&chkid) {
                iff_out.put_chunk(&chkid);
                iff_out.copy_from(chunk_bs)?;
            }
        }
        iff_out.close_chunk()?;
        self.data_pool = DataPool::new(str_out);
        *self.anno.lock().unwrap() = None;
        self.flags.lock().unwrap().insert(Flags::MODIFIED);
    }

    fn reset(&self) {
        *self.info.lock().unwrap() = None;
        *self.bg44.lock().unwrap() = None;
        *self.bgpm.lock().unwrap() = None;
        *self.fgjb.lock().unwrap() = None;
        *self.fgjd.lock().unwrap() = None;
        *self.fgpm.lock().unwrap() = None;
        *self.fgbc.lock().unwrap() = None;
        *self.anno.lock().unwrap() = None;
        *self.text.lock().unwrap() = None;
        *self.meta.lock().unwrap() = None;
        *self.dir.lock().unwrap() = None;
        *self.description.lock().unwrap() = String::new();
        *self.mimetype.lock().unwrap() = String::new();
        let mut flags = self.flags.lock().unwrap();
        *flags = flags.intersection(Flags::ALL_DATA_PRESENT | Flags::DECODE_STOPPED | Flags::DECODE_FAILED);
    }

    fn trigger_cb(&self) {
        *self.file_size.lock().unwrap() = self.data_pool.get_length();
        self.flags.lock().unwrap().insert(Flags::DATA_PRESENT);
        if !self.are_incl_files_created() {
            self.process_incl_chunks();
        }
        if self.inc_files.lock().unwrap().iter().all(|f| f.is_all_data_present()) {
            self.flags.lock().unwrap().insert(Flags::ALL_DATA_PRESENT);
        }
    }

    fn is_decoding(&self) -> bool {
        self.flags.lock().unwrap().contains(Flags::DECODING)
    }

    fn is_decode_ok(&self) -> bool {
        self.flags.lock().unwrap().contains(Flags::DECODE_OK)
    }

    fn is_data_present(&self) -> bool {
        self.flags.lock().unwrap().contains(Flags::DATA_PRESENT)
    }

    fn is_all_data_present(&self) -> bool {
        self.flags.lock().unwrap().contains(Flags::ALL_DATA_PRESENT)
    }

    fn are_incl_files_created(&self) -> bool {
        self.flags.lock().unwrap().contains(Flags::INCL_FILES_CREATED)
    }

    fn process_incl_chunks(&self) {
        let mut bs = self.data_pool.get_stream()?;
        let mut iff = IFFByteStream::new(bs);
        let mut incl_cnt = 0;
        iff.get_chunk()?;
        while let Some((chkid, chunk_bs)) = iff.get_chunk() {
            if chkid == "INCL" {
                self.process_incl_chunk(chunk_bs);
                incl_cnt += 1;
            }
        }
        self.flags.lock().unwrap().insert(Flags::INCL_FILES_CREATED);
    }

    fn handle_decode_error(&self, error: DjVuError) {
        let mut flags = self.flags.lock().unwrap();
        match error {
            DjVuError::DecodingError(_) => {
                *flags = flags.difference(Flags::DECODING).union(Flags::DECODE_FAILED);
            }
            DjVuError::IoError(_) => {
                *flags = flags.difference(Flags::DECODING).union(Flags::DECODE_STOPPED);
            }
        }
    }

    fn get_fgjd(&self) -> Option<JB2Dict> {
        if let Some(fgjd) = self.fgjd.lock().unwrap().as_ref() {
            return Some(fgjd.clone());
        }
        for file in self.get_included_files(true) {
            if let Some(fgjd) = file.get_fgjd() {
                return Some(fgjd);
            }
        }
        None
    }
}

fn is_annotation(chkid: &str) -> bool {
    chkid == "ANTa" || chkid == "ANTz" || chkid == "FORM:ANNO"
}

fn is_text(chkid: &str) -> bool {
    chkid == "TXTa" || chkid == "TXTz"
}

fn is_meta(chkid: &str) -> bool {
    chkid == "METa" || chkid == "METz"
}

// Placeholder types (to be implemented separately)
#[derive(Clone)]
struct Url;
struct DataPool;
struct ByteStream;
struct IFFByteStream;
struct DjVuInfo;
struct IW44Image;
struct GPixmap;
struct JB2Image;
struct JB2Dict;
struct DjVuPalette;
struct DjVuNavDir;
struct DjVuPort;
struct DjVuSimplePort;
struct DjVuPortcaster;

enum ErrorRecoveryAction {
    Abort,
    SkipPages,
}

enum IW44ImageType {
    Color,
}

impl Url {
    fn from_string(s: String) -> Self { Url }
}

impl DataPool {
    fn new(stream: ByteStream) -> Self { DataPool }
    fn empty() -> Self { DataPool }
    fn get_stream(&self) -> Result<ByteStream> { Ok(ByteStream) }
    fn add_trigger(&self, pos: i32, cb: fn(&DjVuFile)) {}
    fn get_length(&self) -> u32 { 0 }
}

impl ByteStream {
    fn new() -> Self { ByteStream }
    fn read_string(&mut self) -> Result<String> { Ok(String::new()) }
    fn write_u8(&mut self, value: u8) -> Result<()> { Ok(()) }
    fn copy_from(&mut self, other: ByteStream) -> Result<()> { Ok(()) }
    fn seek(&mut self, pos: u64) -> Result<()> { Ok(()) }
    fn position(&self) -> u64 { 0 }
}

impl IFFByteStream {
    fn new(bs: ByteStream) -> Self { IFFByteStream }
    fn get_chunk(&mut self) -> Result<Option<(String, ByteStream)>> { Ok(None) }
    fn put_chunk(&mut self, chkid: &str) {}
    fn copy_from(&mut self, bs: ByteStream) -> Result<()> { Ok(()) }
    fn close_chunk(&mut self) -> Result<()> { Ok(()) }
}

impl DjVuInfo {
    fn decode(bs: ByteStream) -> Result<Self> { Ok(DjVuInfo) }
}

impl IW44Image {
    fn decode_chunk(bs: ByteStream, ty: IW44ImageType) -> Result<Self> { Ok(IW44Image) }
    fn refine(&mut self, bs: ByteStream) -> Result<()> { Ok(()) }
}

impl JB2Dict {
    fn decode(bs: ByteStream) -> Result<Self> { Ok(JB2Dict) }
}

impl JB2Image {
    fn decode(bs: ByteStream, fgjd: Option<JB2Dict>) -> Result<Self> { Ok(JB2Image) }
}

impl DjVuPalette {
    fn decode(bs: ByteStream) -> Result<Self> { Ok(DjVuPalette) }
}

impl DjVuNavDir {
    fn decode(bs: ByteStream, url: &Url) -> Result<Self> { Ok(DjVuNavDir) }
}

impl DjVuPortcaster {
    fn global() -> Self { DjVuPortcaster }
    fn request_data(&self, url: &Url) -> Option<DataPool> { Some(DataPool::empty()) }
    fn add_route(&self, file: &DjVuFile, port: DjVuPort) {}
    fn id_to_url(&self, base: &Url, id: &str) -> Result<Url> { Ok(Url) }
    fn id_to_file(&self, base: &Url, id: &str) -> Result<Arc<DjVuFile>> { Ok(Arc::new(DjVuFile::create_from_url(Url, None))) }
}

impl DjVuSimplePort {
    fn new() -> Self { DjVuSimplePort }
}