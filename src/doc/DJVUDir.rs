use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::io::{self, Read, Write};

// Assuming these are defined in a separate module (e.g., byte_stream.rs)
mod byte_stream {
    use std::io::{self, Read, Write};
    #[derive(Debug)]
    pub enum DjVuError {
        DecodingError(String),
        IoError(io::Error),
    }
    impl From<io::Error> for DjVuError {
        fn from(err: io::Error) -> Self {
            DjVuError::IoError(err)
        }
    }
    pub trait ByteStream: Read + Write {
        fn read_u8(&mut self) -> Result<u8, DjVuError>;
        fn read_u16(&mut self) -> Result<u16, DjVuError>;
        fn read_u24(&mut self) -> Result<u32, DjVuError>;
        fn read_u32(&mut self) -> Result<u32, DjVuError>;
        fn write_u8(&mut self, value: u8) -> Result<(), DjVuError>;
        fn write_u16(&mut self, value: u16) -> Result<(), DjVuError>;
        fn write_u24(&mut self, value: u32) -> Result<(), DjVuError>;
        fn write_u32(&mut self, value: u32) -> Result<(), DjVuError>;
        fn writestring(&mut self, s: &str) -> Result<(), DjVuError>;
        // Placeholder for BZZ-compressed stream methods
    }
    // Placeholder implementations would go here
}

// Re-export for convenience
use byte_stream::{ByteStream, DjVuError};
type Result<T> = std::result::Result<T, DjVuError>;

// File types for DjVmDir
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Include = 0,
    Page = 1,
    Thumbnails = 2,
    SharedAnno = 3,
}

/// Represents a file record in a DjVmDir directory
#[derive(Debug, Clone)]
pub struct File {
    pub id: String,         // File identifier
    pub name: String,       // File name for saving
    pub title: String,      // User-friendly title
    pub offset: u32,        // Offset in bundled format
    pub size: u32,          // Size of the file
    pub file_type: FileType,// Type of the file
    pub has_name: bool,     // Indicates if name differs from id
    pub has_title: bool,    // Indicates if title differs from id
    pub page_num: i32,      // Page number if a page, -1 otherwise
    pub valid_name: bool,   // Whether the name is valid for native encoding
    oldname: String,        // Original name before modification
}

impl File {
    /// Creates a new File instance wrapped in an Arc
    pub fn new(id: &str, name: &str, title: &str, file_type: FileType) -> Arc<Self> {
        Arc::new(File {
            id: id.to_string(),
            name: name.to_string(),
            title: title.to_string(),
            offset: 0,
            size: 0,
            file_type,
            has_name: name != id,
            has_title: title != id,
            page_num: -1,
            valid_name: false,
            oldname: String::new(),
        })
    }

    /// Checks and modifies the save name if invalid for native encoding
    pub fn check_save_name(&mut self, is_bundled: bool) -> String {
        if !is_bundled && !self.valid_name {
            let mut retval = if self.name.is_empty() { &self.id } else { &self.name }.to_string();
            // Simplified check for native encoding compatibility
            // In real implementation, check against filesystem encoding
            if retval.chars().any(|c| c.is_control() || c > '\x7F') {
                let mut buf = String::new();
                for c in retval.chars() {
                    if c.is_control() || c > '\x7F' {
                        buf.push_str(&format!("{:02X}", c as u8));
                    } else {
                        buf.push(c);
                    }
                }
                self.oldname = std::mem::replace(&mut self.name, buf);
                self.valid_name = true;
            }
            self.valid_name = true;
            self.name.clone()
        } else {
            self.get_save_name()
        }
    }

    /// Returns the save name (name if set, else id)
    pub fn get_save_name(&self) -> String {
        if self.name.is_empty() { self.id.clone() } else { self.name.clone() }
    }

    /// Returns the load name (id)
    pub fn get_load_name(&self) -> &str {
        &self.id
    }

    /// Sets the load name (id) based on a URL-like string
    pub fn set_load_name(&mut self, id: &str) {
        // Simplified: assumes id is the filename part of a URL
        self.id = id.to_string();
    }

    /// Sets the save name, resetting validity
    pub fn set_save_name(&mut self, name: &str) {
        self.valid_name = false;
        self.name = name.to_string();
        self.oldname = String::new();
    }

    /// Returns the title (title if set, else id)
    pub fn get_title(&self) -> String {
        if self.title.is_empty() { self.id.clone() } else { self.title.clone() }
    }

    /// Sets the title
    pub fn set_title(&mut self, title: &str) {
        self.title = title.to_string();
    }

    /// Returns a string representation of the file type
    pub fn get_str_type(&self) -> String {
        match self.file_type {
            FileType::Include => "INCLUDE".to_string(),
            FileType::Page => "PAGE".to_string(),
            FileType::Thumbnails => "THUMBNAILS".to_string(),
            FileType::SharedAnno => "SHARED_ANNO".to_string(),
        }
    }

    /// Checks if the file is a page
    pub fn is_page(&self) -> bool {
        self.file_type == FileType::Page
    }

    /// Checks if the file is an include file
    pub fn is_include(&self) -> bool {
        self.file_type == FileType::Include
    }

    /// Checks if the file contains thumbnails
    pub fn is_thumbnails(&self) -> bool {
        self.file_type == FileType::Thumbnails
    }

    /// Checks if the file contains shared annotations
    pub fn is_shared_anno(&self) -> bool {
        self.file_type == FileType::SharedAnno
    }

    /// Returns the page number (-1 if not a page)
    pub fn get_page_num(&self) -> i32 {
        self.page_num
    }
}

/// Directory for a multipage DjVu document (DIRM chunk)
pub struct DjVmDir {
    data: Mutex<DjVmDirData>,
}

struct DjVmDirData {
    files_list: Vec<Arc<File>>,
    page2file: Vec<Arc<File>>,
    name2file: HashMap<String, Arc<File>>,
    id2file: HashMap<String, Arc<File>>,
}

impl DjVmDir {
    const VERSION: u8 = 1;

    /// Creates a new DjVmDir instance wrapped in an Arc
    pub fn new() -> Arc<Self> {
        Arc::new(DjVmDir {
            data: Mutex::new(DjVmDirData {
                files_list: Vec::new(),
                page2file: Vec::new(),
                name2file: HashMap::new(),
                id2file: HashMap::new(),
            }),
        })
    }

    /// Decodes the directory from a ByteStream
    pub fn decode(&self, stream: &mut dyn ByteStream) -> Result<()> {
        let mut data = self.data.lock().unwrap();
        data.files_list.clear();
        data.page2file.clear();
        data.name2file.clear();
        data.id2file.clear();

        let ver = stream.read_u8()?;
        let bundled = (ver & 0x80) != 0;
        let version = ver & 0x7f;
        if version > Self::VERSION {
            return Err(DjVuError::DecodingError(format!(
                "Unsupported DIRM version: {}", version
            )));
        }
        let files_count = stream.read_u16()?;
        let mut files = Vec::with_capacity(files_count as usize);

        if bundled {
            for _ in 0..files_count {
                let offset = stream.read_u32()?;
                if offset == 0 && version > 0 {
                    return Err(DjVuError::DecodingError("Zero offset in bundled format".into()));
                }
                let mut file = File::new("", "", "", FileType::Include);
                Arc::get_mut(&mut file).unwrap().offset = offset;
                if version == 0 {
                    Arc::get_mut(&mut file).unwrap().size = stream.read_u24()?;
                }
                files.push(file);
            }
        } else {
            for _ in 0..files_count {
                files.push(File::new("", "", "", FileType::Include));
            }
        }

        // Assuming a BZZ decoder is available; for now, simulate reading
        // In practice, replace with actual BZZ decoding logic
        if version > 0 {
            for file in &mut files {
                Arc::get_mut(file).unwrap().size = stream.read_u24()?;
            }
        }

        for file in &mut files {
            let flags = stream.read_u8()?;
            let file_type = match flags & 0x3f {
                0 => FileType::Include,
                1 => FileType::Page,
                2 => FileType::Thumbnails,
                3 => FileType::SharedAnno,
                _ => return Err(DjVuError::DecodingError("Invalid file type".into())),
            };
            let mut file = Arc::get_mut(file).unwrap();
            file.file_type = file_type;
            file.has_name = (flags & 0x80) != 0;
            file.has_title = (flags & 0x40) != 0;
        }

        // Read strings (simplified; assumes null-terminated strings in stream)
        let mut strings = Vec::new();
        let mut buffer = [0u8; 1024];
        while let Ok(len) = stream.read(&mut buffer) {
            if len == 0 { break; }
            strings.extend_from_slice(&buffer[..len]);
        }
        let mut ptr = 0;
        for file in &mut files {
            let mut file = Arc::get_mut(file).unwrap();
            let id_start = ptr;
            while ptr < strings.len() && strings[ptr] != 0 { ptr += 1; }
            file.id = String::from_utf8_lossy(&strings[id_start..ptr]).into_owned();
            ptr += 1;

            if file.has_name {
                let name_start = ptr;
                while ptr < strings.len() && strings[ptr] != 0 { ptr += 1; }
                file.name = String::from_utf8_lossy(&strings[name_start..ptr]).into_owned();
                ptr += 1;
            } else {
                file.name = file.id.clone();
            }

            if file.has_title {
                let title_start = ptr;
                while ptr < strings.len() && strings[ptr] != 0 { ptr += 1; }
                file.title = String::from_utf8_lossy(&strings[title_start..ptr]).into_owned();
                ptr += 1;
            } else {
                file.title = file.id.clone();
            }
        }

        // Populate data structures
        data.files_list = files.clone();
        let mut page_num = 0;
        for file in &files {
            if file.is_page() {
                Arc::get_mut(file).unwrap().page_num = page_num;
                data.page2file.push(Arc::clone(file));
                page_num += 1;
            }
            if data.name2file.contains_key(&file.name) {
                return Err(DjVuError::DecodingError(format!("Duplicate name: {}", file.name)));
            }
            data.name2file.insert(file.name.clone(), Arc::clone(file));
            if data.id2file.contains_key(&file.id) {
                return Err(DjVuError::DecodingError(format!("Duplicate ID: {}", file.id)));
            }
            data.id2file.insert(file.id.clone(), Arc::clone(file));
        }

        let shared_anno_count = files.iter().filter(|f| f.is_shared_anno()).count();
        if shared_anno_count > 1 {
            return Err(DjVuError::DecodingError("Multiple shared annotation files".into()));
        }

        Ok(())
    }

    /// Encodes the directory to a ByteStream
    pub fn encode(&self, stream: &mut dyn ByteStream, do_rename: bool) -> Result<()> {
        let data = self.data.lock().unwrap();
        let bundled = data.files_list.iter().all(|f| f.offset > 0);
        if data.files_list.iter().any(|f| (f.offset > 0) != bundled) {
            return Err(DjVuError::DecodingError("Mixed bundled and indirect records".into()));
        }
        self.encode_explicit(stream, bundled, do_rename)
    }

    /// Encodes the directory with explicit bundled/indirect specification
    pub fn encode_explicit(&self, stream: &mut dyn ByteStream, bundled: bool, do_rename: bool) -> Result<()> {
        let data = self.data.lock().unwrap();
        stream.write_u8(Self::VERSION | if bundled { 0x80 } else { 0 })?;
        stream.write_u16(data.files_list.len() as u16)?;

        if data.files_list.is_empty() {
            return Ok(());
        }

        let shared_anno_count = data.files_list.iter().filter(|f| f.is_shared_anno()).count();
        if shared_anno_count > 1 {
            return Err(DjVuError::DecodingError("Multiple shared annotation files".into()));
        }

        if bundled {
            for file in &data.files_list {
                if file.offset == 0 {
                    return Err(DjVuError::DecodingError("Missing offset in bundled format".into()));
                }
                stream.write_u32(file.offset)?;
            }
        }

        // BZZ encoding simulation; replace with actual BZZ encoder
        for file in &data.files_list {
            stream.write_u24(file.size)?;
        }

        let do_rename = do_rename || !bundled;
        for file in &data.files_list {
            let mut flags = file.file_type as u8;
            if do_rename {
                if file.name.is_empty() || file.oldname == file.name {
                    flags &= !0x80;
                } else {
                    flags |= 0x80;
                }
            } else {
                if file.name.is_empty() || file.name == file.id {
                    flags &= !0x80;
                } else {
                    flags |= 0x80;
                }
            }
            if !file.title.is_empty() && file.title != file.id {
                flags |= 0x40;
            } else {
                flags &= !0x40;
            }
            stream.write_u8(flags)?;

            let (id, name, title) = if do_rename {
                let id = if file.name.is_empty() { &file.id } else { &file.name };
                let name = if (flags & 0x80) != 0 { &file.oldname } else { "" };
                (id, name, if (flags & 0x40) != 0 { &file.title } else { "" })
            } else {
                let name = if (flags & 0x80) != 0 { &file.name } else { "" };
                (&file.id, name, if (flags & 0x40) != 0 { &file.title } else { "" })
            };
            stream.writestring(id)?;
            stream.write_u8(0)?;
            if !name.is_empty() {
                stream.writestring(name)?;
                stream.write_u8(0)?;
            }
            if !title.is_empty() {
                stream.writestring(title)?;
                stream.write_u8(0)?;
            }
        }

        Ok(())
    }

    /// Checks if the directory is indirect
    pub fn is_indirect(&self) -> bool {
        let data = self.data.lock().unwrap();
        !data.files_list.is_empty() && data.files_list[0].offset == 0
    }

    /// Checks if the directory is bundled
    pub fn is_bundled(&self) -> bool {
        !self.is_indirect()
    }

    /// Retrieves a file by page number
    pub fn page_to_file(&self, page_num: i32) -> Option<Arc<File>> {
        let data = self.data.lock().unwrap();
        if page_num >= 0 && (page_num as usize) < data.page2file.len() {
            Some(Arc::clone(&data.page2file[page_num as usize]))
        } else {
            None
        }
    }

    /// Retrieves a file by name
    pub fn name_to_file(&self, name: &str) -> Option<Arc<File>> {
        let data = self.data.lock().unwrap();
        data.name2file.get(name).cloned()
    }

    /// Retrieves a file by ID
    pub fn id_to_file(&self, id: &str) -> Option<Arc<File>> {
        let data = self.data.lock().unwrap();
        data.id2file.get(id).cloned()
    }

    /// Retrieves a file by title (first match among pages)
    pub fn title_to_file(&self, title: &str) -> Option<Arc<File>> {
        let data = self.data.lock().unwrap();
        data.files_list.iter()
            .find(|f| f.is_page() && f.get_title() == title)
            .cloned()
    }

    /// Retrieves a file by position in the files list
    pub fn pos_to_file(&self, fileno: i32) -> Option<(Arc<File>, Option<i32>)> {
        let data = self.data.lock().unwrap();
        if fileno < 0 || fileno as usize >= data.files_list.len() {
            return None;
        }
        let mut pageno = 0;
        for (i, file) in data.files_list.iter().enumerate() {
            if i == fileno as usize {
                return Some((Arc::clone(file), if file.is_page() { Some(pageno) } else { None }));
            }
            if file.is_page() {
                pageno += 1;
            }
        }
        None
    }

    /// Gets the position of a file in the files list
    pub fn get_file_pos(&self, file: &File) -> Option<usize> {
        let data = self.data.lock().unwrap();
        data.files_list.iter().position(|f| Arc::ptr_eq(f, &Arc::new(file.clone())))
    }

    /// Gets the position of a page in the files list
    pub fn get_page_pos(&self, page_num: i32) -> Option<usize> {
        let file = self.page_to_file(page_num)?;
        self.get_file_pos(&file)
    }

    /// Resolves duplicate names by modifying them
    pub fn resolve_duplicates(&self, save_as_bundled: bool) -> Vec<Arc<File>> {
        let mut data = self.data.lock().unwrap();
        let mut save_map = HashMap::new();
        let mut conflicts = HashMap::new();

        for file in &data.files_list {
            let save_name = Arc::get_mut(&mut Arc::clone(file)).unwrap()
                .check_save_name(save_as_bundled)
                .to_lowercase();
            if save_map.contains_key(&save_name) {
                conflicts.entry(save_name.clone())
                    .or_insert_with(Vec::new)
                    .push(Arc::clone(file));
            } else {
                save_map.insert(save_name, ());
            }
        }

        for (save_name, cfiles) in conflicts {
            let dot = save_name.rfind('.');
            let mut count = 1;
            for file in cfiles {
                let mut file = Arc::get_mut(&mut Arc::clone(&file)).unwrap();
                let mut new_name = file.get_load_name().to_string();
                while save_map.contains_key(&new_name.to_lowercase()) {
                    new_name = if let Some(dot) = dot {
                        format!("{}-{}{}", &save_name[..dot], count, &save_name[dot..])
                    } else {
                        format!("{}-{}", save_name, count)
                    };
                    count += 1;
                }
                file.set_save_name(&new_name);
                save_map.insert(new_name.to_lowercase(), ());
            }
        }
        data.files_list.clone()
    }

    /// Returns a list of all files
    pub fn get_files_list(&self) -> Vec<Arc<File>> {
        let data = self.data.lock().unwrap();
        data.files_list.clone()
    }

    /// Returns the number of files
    pub fn get_files_num(&self) -> usize {
        let data = self.data.lock().unwrap();
        data.files_list.len()
    }

    /// Returns the number of pages
    pub fn get_pages_num(&self) -> usize {
        let data = self.data.lock().unwrap();
        data.page2file.len()
    }

    /// Retrieves the shared annotation file, if any
    pub fn get_shared_anno_file(&self) -> Option<Arc<File>> {
        let data = self.data.lock().unwrap();
        data.files_list.iter().find(|f| f.is_shared_anno()).cloned()
    }

    /// Sets the title of a file by ID
    pub fn set_file_title(&self, id: &str, title: &str) -> Result<()> {
        let mut data = self.data.lock().unwrap();
        let file = data.id2file.get_mut(id)
            .ok_or_else(|| DjVuError::DecodingError(format!("No file with ID: {}", id)))?;
        Arc::get_mut(file).unwrap().set_title(title);
        Ok(())
    }

    /// Sets the name of a file by ID
    pub fn set_file_name(&self, id: &str, name: &str) -> Result<()> {
        let mut data = self.data.lock().unwrap();
        for file in &data.files_list {
            if file.id != id && file.name == name {
                return Err(DjVuError::DecodingError(format!("Name already in use: {}", name)));
            }
        }
        let file = data.id2file.get_mut(id)
            .ok_or_else(|| DjVuError::DecodingError(format!("No file with ID: {}", id)))?;
        let old_name = file.name.clone();
        Arc::get_mut(file).unwrap().set_save_name(name);
        data.name2file.remove(&old_name);
        data.name2file.insert(name.to_string(), Arc::clone(file));
        Ok(())
    }

    /// Inserts a file at the specified position (-1 for append)
    pub fn insert_file(&self, file: Arc<File>, pos: i32) -> Result<i32> {
        let mut data = self.data.lock().unwrap();
        let pos = if pos < 0 { data.files_list.len() as i32 } else { pos };

        if data.id2file.contains_key(&file.id) {
            return Err(DjVuError::DecodingError(format!("Duplicate ID: {}", file.id)));
        }
        if data.name2file.contains_key(&file.name) {
            return Err(DjVuError::DecodingError(format!("Duplicate name: {}", file.name)));
        }
        if file.is_shared_anno() && data.files_list.iter().any(|f| f.is_shared_anno()) {
            return Err(DjVuError::DecodingError("Multiple shared annotation files".into()));
        }

        data.name2file.insert(file.name.clone(), Arc::clone(&file));
        data.id2file.insert(file.id.clone(), Arc::clone(&file));

        if (pos as usize) <= data.files_list.len() {
            data.files_list.insert(pos as usize, Arc::clone(&file));
        } else {
            data.files_list.push(Arc::clone(&file));
        }

        if file.is_page() {
            let mut page_num = 0;
            for f in &data.files_list {
                if Arc::ptr_eq(f, &file) {
                    break;
                }
                if f.is_page() {
                    page_num += 1;
                }
            }
            let mut file_mut = Arc::get_mut(&mut Arc::clone(&file)).unwrap();
            file_mut.page_num = page_num;
            data.page2file.insert(page_num as usize, Arc::clone(&file));
            for i in page_num as usize..data.page2file.len() {
                Arc::get_mut(&mut data.page2file[i]).unwrap().page_num = i as i32;
            }
        }

        Ok(pos)
    }

    /// Deletes a file by ID
    pub fn delete_file(&self, id: &str) -> Result<()> {
        let mut data = self.data.lock().unwrap();
        if let Some(pos) = data.files_list.iter().position(|f| f.id == id) {
            let file = data.files_list.remove(pos);
            data.name2file.remove(&file.name);
            data.id2file.remove(&file.id);
            if file.is_page() {
                let page_pos = data.page2file.iter().position(|f| Arc::ptr_eq(f, &file)).unwrap();
                data.page2file.remove(page_pos);
                for i in page_pos..data.page2file.len() {
                    Arc::get_mut(&mut data.page2file[i]).unwrap().page_num = i as i32;
                }
            }
            Ok(())
        } else {
            Err(DjVuError::DecodingError(format!("No file with ID: {}", id)))
        }
    }
}

/// Directory for an older DjVu all-in-one-file format (DIR0 chunk)
pub struct DjVmDir0 {
    name2file: HashMap<String, Arc<FileRec>>,
    num2file: Vec<Arc<FileRec>>,
}

#[derive(Debug, Clone)]
pub struct FileRec {
    pub name: String,
    pub iff_file: bool,
    pub offset: u32,
    pub size: u32,
}

impl FileRec {
    pub fn new(name: &str, iff_file: bool, offset: u32, size: u32) -> Arc<Self> {
        Arc::new(FileRec {
            name: name.to_string(),
            iff_file,
            offset,
            size,
        })
    }
}

impl DjVmDir0 {
    /// Creates a new DjVmDir0 instance
    pub fn new() -> Arc<Self> {
        Arc::new(DjVmDir0 {
            name2file: HashMap::new(),
            num2file: Vec::new(),
        })
    }

    /// Calculates the encoded size of the directory
    pub fn get_size(&self) -> usize {
        2 + self.num2file.iter().map(|f| f.name.len() + 1 + 1 + 4 + 4).sum::<usize>()
    }

    /// Encodes the directory to a ByteStream
    pub fn encode(&self, stream: &mut dyn ByteStream) -> Result<()> {
        stream.write_u16(self.num2file.len() as u16)?;
        for file in &self.num2file {
            stream.writestring(&file.name)?;
            stream.write_u8(0)?;
            stream.write_u8(if file.iff_file { 1 } else { 0 })?;
            stream.write_u32(file.offset)?;
            stream.write_u32(file.size)?;
        }
        Ok(())
    }

    /// Decodes the directory from a ByteStream
    pub fn decode(&mut self, stream: &mut dyn ByteStream) -> Result<()> {
        self.name2file.clear();
        self.num2file.clear();

        let count = stream.read_u16()?;
        for _ in 0..count {
            let mut name = String::new();
            let mut byte = stream.read_u8()?;
            while byte != 0 {
                name.push(byte as char);
                byte = stream.read_u8()?;
            }
            let iff_file = stream.read_u8()? != 0;
            let offset = stream.read_u32()?;
            let size = stream.read_u32()?;
            self.add_file(&name, iff_file, offset, size)?;
        }
        Ok(())
    }

    /// Retrieves a file by name
    pub fn get_file_by_name(&self, name: &str) -> Option<Arc<FileRec>> {
        self.name2file.get(name).cloned()
    }

    /// Retrieves a file by index
    pub fn get_file_by_num(&self, file_num: usize) -> Option<Arc<FileRec>> {
        self.num2file.get(file_num).cloned()
    }

    /// Adds a file to the directory
    pub fn add_file(&mut self, name: &str, iff_file: bool, offset: u32, size: u32) -> Result<()> {
        if name.contains('/') {
            return Err(DjVuError::DecodingError("File name cannot contain slashes".into()));
        }
        let file = FileRec::new(name, iff_file, offset, size);
        self.name2file.insert(name.to_string(), Arc::clone(&file));
        self.num2file.push(file);
        Ok(())
    }
}