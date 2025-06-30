use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use url::Url; // Assuming a URL crate like `url` for handling URLs

/// Represents a multipage DjVu document for encoding purposes.
#[derive(Debug)]
pub struct DjVuDocument {
    dir: DjVmDir,
    data: HashMap<String, DataPool>,
    nav: Option<DjVmNav>,
}

/// Directory of files in a DjVu multipage document.
#[derive(Debug, Clone)]
struct DjVmDir {
    files: Vec<FileRecord>,
}

/// A single file record in the DjVu document directory.
#[derive(Debug, Clone)]
pub struct FileRecord {
    pub file_type: FileType,
    pub file_id: String,
    pub file_name: String,
    pub file_title: String,
    pub file_size: usize,
    pub is_included: bool,
}

/// Type of file in the DjVu document.
#[derive(Debug, Clone, PartialEq)]
pub enum FileType {
    Page,
    Include,      // e.g., shared dictionaries or annotations
    SharedAnno,   // Shared annotations
    Thumbnails,
}

/// Navigation/bookmark data (simplified for encoding).
#[derive(Debug, Clone)]
struct DjVmNav {
    // Placeholder for bookmark data; extend as needed
    bookmarks: Vec<String>,
}

/// Data pool abstraction for file contents.
#[derive(Debug, Clone)]
struct DataPool {
    data: Vec<u8>,
}

impl DjVuDocument {
    /// Creates a new, empty DjVu document.
    pub fn new() -> Self {
        DjVuDocument {
            dir: DjVmDir::new(),
            data: HashMap::new(),
            nav: None,
        }
    }

    /// Inserts a file into the document with its data.
    pub fn insert_file(&mut self, file: FileRecord, data: Vec<u8>) -> Result<(), io::Error> {
        if self.data.contains_key(&file.id) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("Duplicate file ID: {}", file.id),
            ));
        }
        self.data.insert(file.id.clone(), DataPool { data });
        self.dir.insert_file(file);
        Ok(())
    }

    /// Inserts a file and recursively includes its dependencies (e.g., dictionaries).
    pub fn insert_file_with_includes<F>(
        &mut self,
        file: FileRecord,
        data: Vec<u8>,
        get_data: F,
    ) -> Result<(), io::Error>
    where
        F: Fn(&str) -> Option<Vec<u8>>,
    {
        let mut to_insert = vec![(file, data)];
        let mut inserted = HashSet::new();

        while let Some((file, data)) = to_insert.pop() {
            let id = file.id.clone();
            if inserted.contains(&id) {
                continue;
            }

            self.insert_file(file.clone(), data)?;
            inserted.insert(id.clone());

            let included_ids = self.get_included_ids(&self.data[&id])?;
            for incl_id in included_ids {
                if !inserted.contains(&incl_id) {
                    if let Some(incl_data) = get_data(&incl_id) {
                        let incl_file = FileRecord::new(
                            incl_id.clone(),
                            incl_id.clone(),
                            "".to_string(),
                            FileType::Include,
                        );
                        to_insert.push((incl_file, incl_data));
                    } else {
                        return Err(io::Error::new(
                            io::ErrorKind::NotFound,
                            format!("Included file not provided: {}", incl_id),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Sets the document's bookmarks.
    pub fn set_bookmarks(&mut self, bookmarks: Vec<String>) {
        self.nav = Some(DjVmNav { bookmarks });
    }

    /// Writes the document in bundled format to the provided writer.
    pub fn write_bundled<W: Write + Seek>(&self, mut writer: W) -> Result<(), io::Error> {
        let mut iff = IffWriter::new(&mut writer);

        // Write FORM:DJVM header
        iff.put_chunk("FORM:DJVM")?;

        // Encode DIRM with dummy offsets
        let mut dirm_buffer = Vec::new();
        self.dir.encode(&mut dirm_buffer, true)?;
        let dirm_size = dirm_buffer.len();

        // Encode NAVM if present
        let mut nav_buffer = Vec::new();
        if let Some(nav) = &self.nav {
            nav.encode(&mut nav_buffer)?;
        }
        let nav_size = nav_buffer.len();

        // Calculate starting offset for file data
        let mut offset = 12 + 8 + dirm_size; // FORM:DJVM + DIRM chunk
        if nav_size > 0 {
            offset += 8 + nav_size; // NAVM chunk
        }

        // Update file offsets
        let mut files = self.dir.files.clone();
        for file in &mut files {
            if offset % 2 != 0 {
                offset += 1;
            }
            file.offset = offset as u32;
            file.size = self.data[&file.id].data.len() as u32;
            offset += file.size as usize;
        }

        // Write real DIRM chunk
        let mut real_dirm_buffer = Vec::new();
        let temp_dir = DjVmDir { files: files.clone() };
        temp_dir.encode(&mut real_dirm_buffer, false)?;
        iff.put_chunk("DIRM")?;
        iff.write_all(&real_dirm_buffer)?;
        iff.close_chunk()?;

        // Write NAVM chunk if present
        if let Some(_) = &self.nav {
            iff.put_chunk("NAVM")?;
            iff.write_all(&nav_buffer)?;
            iff.close_chunk()?;
        }

        // Write file data
        for file in &files {
            if iff.tell() % 2 != 0 {
                iff.write_all(&[0])?;
            }
            iff.write_all(&self.data[&file.id].data)?;
        }

        iff.close_chunk()?;
        Ok(())
    }

    /// Writes the document in indirect format to the specified directory.
    pub fn write_indirect(&self, codebase: &Url, idx_name: &str) -> Result<(), io::Error> {
        let files = self.dir.resolve_duplicates();

        // Write each file with remapped INCL chunks
        for file in &files {
            let name = file.name.clone();
            let url = Url::parse(&format!("{}/{}", codebase, name))
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            let path = Path::new(url.path());
            let mut writer = File::create(path)?;
            self.save_file_with_remap(&self.data[&file.id], &mut writer)?;
        }

        // Write index file if requested
        if !idx_name.is_empty() {
            let idx_url = Url::parse(&format!("{}/{}", codebase, idx_name))
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
            let mut writer = File::create(idx_url.path())?;
            let mut iff = IffWriter::new(&mut writer);
            iff.put_chunk("FORM:DJVM")?;
            iff.put_chunk("DIRM")?;
            self.dir.encode(&mut iff, false)?;
            iff.close_chunk()?;
            if let Some(nav) = &self.nav {
                iff.put_chunk("NAVM")?;
                nav.encode(&mut iff)?;
                iff.close_chunk()?;
            }
            iff.close_chunk()?;
        }
        Ok(())
    }

    /// Parses IFF structure to extract included file IDs from INCL chunks.
    fn get_included_ids(&self, data_pool: &DataPool) -> Result<Vec<String>, io::Error> {
        let mut ids = Vec::new();
        let mut reader = IffReader::new(Cursor::new(&data_pool.data));
        while let Some(chunk) = reader.next_chunk()? {
            if chunk.id == "INCL" {
                let data = reader.get_chunk_data(&chunk)?;
                let id = String::from_utf8_lossy(data).trim().to_string();
                ids.push(id);
            }
        }
        Ok(ids)
    }

    /// Saves a file with INCL chunks remapped according to the directory.
    fn save_file_with_remap<W: Write + Seek>(
        &self,
        data_pool: &DataPool,
        writer: &mut W,
    ) -> Result<(), io::Error> {
        let mut reader = IffReader::new(Cursor::new(&data_pool.data));
        let mut iff_writer = IffWriter::new(writer);

        while let Some(chunk) = reader.next_chunk()? {
            iff_writer.put_chunk(&chunk.id)?;
            if chunk.id == "INCL" {
                let incl_id = String::from_utf8_lossy(reader.get_chunk_data(&chunk)?)
                    .trim()
                    .to_string();
                if let Some(file) = self.dir.id_to_file(&incl_id) {
                    iff_writer.write_all(file.name.as_bytes())?;
                } else {
                    iff_writer.write_all(reader.get_chunk_data(&chunk)?)?;
                }
            } else {
                iff_writer.write_all(reader.get_chunk_data(&chunk)?)?;
            }
            iff_writer.close_chunk()?;
        }
        Ok(())
    }
}

impl DjVmDir {
    fn new() -> Self {
        DjVmDir { files: Vec::new() }
    }

    fn insert_file(&mut self, file: FileRecord) {
        self.files.push(file);
    }

    fn id_to_file(&self, id: &str) -> Option<&FileRecord> {
        self.files.iter().find(|f| f.id == id)
    }

    fn resolve_duplicates(&self) -> Vec<FileRecord> {
        let mut files = self.files.clone();
        let mut names = HashSet::new();
        for file in &mut files {
            let mut name = file.name.clone();
            let mut counter = 1;
            while names.contains(&name) {
                if let Some(dot) = name.rfind('.') {
                    name = format!("{}{}{}", &name[..dot], counter, &name[dot..]);
                } else {
                    name = format!("{}_{}", file.name, counter);
                }
                counter += 1;
            }
            file.name = name.clone();
            names.insert(name);
        }
        files
    }

    fn encode<W: Write>(&self, writer: &mut W, dummy_offsets: bool) -> Result<(), io::Error> {
        // Simplified encoding; in practice, encode as per DjVu spec
        let is_bundled = !dummy_offsets; // True for bundled, false for indirect
        writer.write_all(&[if is_bundled { 1 } else { 0 }])?; // Bundled flag
        writer.write_all(&(self.files.len() as u32).to_be_bytes())?;
        for file in &self.files {
            let offset = if dummy_offsets { 0xffffffff } else { file.offset };
            writer.write_all(&offset.to_be_bytes())?;
            writer.write_all(&file.size.to_be_bytes())?;
            let id = file.id.as_bytes();
            writer.write_all(&(id.len() as u8).to_be_bytes())?;
            writer.write_all(id)?;
            let name = file.name.as_bytes();
            writer.write_all(&(name.len() as u8).to_be_bytes())?;
            writer.write_all(name)?;
            let title = file.title.as_bytes();
            writer.write_all(&(title.len() as u8).to_be_bytes())?;
            writer.write_all(title)?;
            writer.write_all(&[file.file_type as u8])?;
        }
        Ok(())
    }
}

impl FileRecord {
    fn new(id: String, name: String, title: String, file_type: FileType) -> Self {
        FileRecord {
            id,
            name,
            title,
            file_type,
            offset: 0,
            size: 0,
        }
    }
}

impl DjVmNav {
    fn encode<W: Write>(&self, writer: &mut W) -> Result<(), io::Error> {
        // Simplified encoding; extend as per DjVu bookmark spec
        writer.write_all(&(self.bookmarks.len() as u32).to_be_bytes())?;
        for bookmark in &self.bookmarks {
            let bytes = bookmark.as_bytes();
            writer.write_all(&(bytes.len() as u16).to_be_bytes())?;
            writer.write_all(bytes)?;
        }
        Ok(())
    }
}

impl DataPool {
    fn new(data: Vec<u8>) -> Self {
        DataPool { data }
    }
}

/// Utility for writing IFF chunks.
pub struct IffWriter<W: Write + Seek> {
    writer: W,
    stack: Vec<u64>, // Positions of chunk size fields
}

impl<W: Write + Seek> IffWriter<W> {
    fn new(writer: W) -> Self {
        IffWriter {
            writer,
            stack: Vec::new(),
        }
    }

    fn put_chunk(&mut self, id: &str) -> Result<(), io::Error> {
        self.writer.write_all(id.as_bytes())?;
        self.stack.push(self.writer.stream_position()?);
        self.writer.write_all(&[0; 4])?; // Placeholder for size
        Ok(())
    }

    fn close_chunk(&mut self) -> Result<(), io::Error> {
        let end_pos = self.writer.stream_position()?;
        let start_pos = self.stack.pop().unwrap();
        let size = (end_pos - start_pos - 4) as u32;
        self.writer.seek(SeekFrom::Start(start_pos))?;
        self.writer.write_all(&size.to_be_bytes())?;
        self.writer.seek(SeekFrom::Start(end_pos))?;
        Ok(())
    }

    fn write_all(&mut self, data: &[u8]) -> Result<(), io::Error> {
        self.writer.write_all(data)?;
        Ok(())
    }

    fn tell(&self) -> u64 {
        self.writer.stream_position().unwrap_or(0)
    }
}

/// Utility for reading IFF chunks (minimal, for include parsing).
pub struct IffReader<R: Read + Seek> {
    reader: R,
}

impl<R: Read + Seek> IffReader<R> {
    fn new(reader: R) -> Self {
        IffReader { reader }
    }

    fn next_chunk(&mut self) -> Result<Option<IffChunk>, io::Error> {
        let mut id = [0; 4];
        if self.reader.read_exact(&mut id).is_err() {
            return Ok(None); // EOF
        }
        let mut size_bytes = [0; 4];
        self.reader.read_exact(&mut size_bytes)?;
        let size = u32::from_be_bytes(size_bytes);
        let offset = self.reader.stream_position()?;
        self.reader.seek(SeekFrom::Current(size as i64))?;
        Ok(Some(IffChunk {
            id: String::from_utf8(id.to_vec()).unwrap_or_default(),
            size,
            data_offset: offset,
        }))
    }

    fn get_chunk_data(&mut self, chunk: &IffChunk) -> Result<&[u8], io::Error> {
        self.reader.seek(SeekFrom::Start(chunk.data_offset))?;
        let mut buffer = vec![0; chunk.size as usize];
        self.reader.read_exact(&mut buffer)?;
        Ok(&buffer) // Note: In a real impl, manage this memory better
    }
}

#[derive(Debug)]
struct IffChunk {
    id: String,
    size: u32,
    data_offset: u64,
}

// Example usage
fn main() -> Result<(), io::Error> {
    let mut doc = DjVuDocument::new();

    // Add a page
    let page_data = vec![/* OCR'd page data */];
    let page = FileRecord::new(
        "page1".to_string(),
        "page1.djvu".to_string(),
        "Page 1".to_string(),
        FileType::Page,
    );
    doc.insert_file(page, page_data)?;

    // Add a shared dictionary
    let dict_data = vec![/* Dictionary data */];
    doc.insert_file_with_includes(
        FileRecord::new(
            "dict".to_string(),
            "dict.djvu".to_string(),
            "".to_string(),
            FileType::Include,
        ),
        dict_data,
        |id| Some(vec![/* Fetch additional include data if needed */]),
    )?;

    // Set bookmarks
    doc.set_bookmarks(vec!["Page 1".to_string()]);

    // Write as bundled
    let mut file = File::create("output.djvu")?;
    doc.write_bundled(&mut file)?;

    // Write as indirect
    let codebase = Url::parse("file:///tmp/djvu/")?;
    doc.write_indirect(&codebase, "index.djvu")?;

    Ok(())
}