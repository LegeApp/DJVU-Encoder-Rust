// src/doc_editor.rs

use crate::doc::djvu_document::{DjVuDocument, FileRecord, FileType};
use crate::iff::data_pool::DataPool;
use crate::doc::djvu_document::{IffReader, IffWriter};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Cursor, Seek};

/// An editor for creating and modifying a `DjVuDocument`.
/// This struct implements the builder pattern for constructing a document
/// before it is finalized and written.
pub struct DjVuDocEditor {
    doc: DjVuDocument,
}

impl DjVuDocEditor {
    /// Creates a new, empty document editor.
    pub fn new() -> Self {
        DjVuDocEditor {
            doc: DjVuDocument::new(),
        }
    }

    /// Finalizes the editing process and returns the constructed document.
    pub fn build(self) -> DjVuDocument {
        self.doc
    }
    
    /// Inserts a page into the document at a specific position.
    ///
    /// # Arguments
    /// * `page_num` - The zero-based index to insert the page at. If negative or out of bounds,
    ///   the page is appended to the end.
    /// * `file` - The `FileRecord` for the page.
    /// * `data` - The raw byte data for the page file.
    /// * `get_include_data` - A closure that can provide the data for any included files
    ///   (e.g., shared dictionaries) referenced by an `INCL` chunk.
    pub fn insert_page<F>(
        &mut self,
        page_num: i32,
        file: FileRecord,
        data: Vec<u8>,
        mut get_include_data: F,
    ) -> Result<(), io::Error>
    where
        F: FnMut(&str) -> Option<Vec<u8>>,
    {
        let mut work_queue: VecDeque<(FileRecord, Vec<u8>)> = VecDeque::new();
        work_queue.push_back((file, data));
        
        let mut processed_ids = HashSet::new();

        while let Some((current_file, current_data)) = work_queue.pop_front() {
            if self.doc.has_file_with_id(&current_file.id) {
                continue; // Already exists, skip.
            }

            // Find and queue dependencies first
            let included_ids = self.parse_included_ids(&current_data)?;
            for incl_id in included_ids {
                if !self.doc.has_file_with_id(&incl_id) && !processed_ids.contains(&incl_id) {
                    if let Some(incl_data) = get_include_data(&incl_id) {
                        let incl_file = FileRecord::new(
                            incl_id.clone(),
                            incl_id.clone(), // Name defaults to ID initially
                            String::new(),
                            FileType::Include,
                        );
                        work_queue.push_back((incl_file, incl_data));
                    } else {
                        return Err(io::Error::new(io::ErrorKind::NotFound, format!("Data for included file '{}' not found", incl_id)));
                    }
                }
            }

            // After parsing its dependencies, add the current file.
            self.doc.insert_file(current_file.clone(), DataPool::new(current_data));
            processed_ids.insert(current_file.id);
        }

        // Reorder the main page to its final position if needed
        self.move_page_by_id(&processed_ids.iter().next().unwrap(), page_num)?;

        Ok(())
    }

    /// Removes a page from the document. If `remove_unreferenced` is true,
    /// any files that were only included by this page (and its children) will also be removed.
    pub fn remove_page(&mut self, page_num: i32, remove_unreferenced: bool) -> Result<(), io::Error> {
        let page_id = self.doc.dir.page_to_id(page_num)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Page not found"))?;

        let mut to_remove = VecDeque::new();
        to_remove.push_back(page_id);
        
        if !remove_unreferenced {
            // Simple case: just remove the page file itself
            self.doc.remove_file(&to_remove[0]);
            return Ok(());
        }

        // Complex case: remove page and any newly unreferenced files
        let ref_map = self.build_ref_map()?;
        let mut parents = ref_map.parents;
        let mut children = ref_map.children;

        while let Some(id_to_remove) = to_remove.pop_front() {
            // For each child of the file we are removing...
            if let Some(child_ids) = children.remove(&id_to_remove) {
                for child_id in child_ids {
                    // ...remove the current file from the child's parent list.
                    if let Some(parent_set) = parents.get_mut(&child_id) {
                        parent_set.remove(&id_to_remove);
                        // If the child has no more parents, it's unreferenced and should be removed.
                        if parent_set.is_empty() {
                            to_remove.push_back(child_id.clone());
                        }
                    }
                }
            }
            // Finally, remove the file itself.
            self.doc.remove_file(&id_to_remove);
        }
        
        Ok(())
    }

    /// Moves a page from one position to another.
    pub fn move_page(&mut self, from_page_num: i32, to_page_num: i32) -> Result<(), io::Error> {
        let id = self.doc.dir.page_to_id(from_page_num)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Source page not found"))?;
        self.move_page_by_id(&id, to_page_num)
    }

    /// Sets the title for a specific page.
    pub fn set_page_title(&mut self, page_num: i32, title: &str) -> Result<(), io::Error> {
        self.doc.dir.set_page_title(page_num, title)
    }

    /// Creates a shared annotation file and includes it in every page.
    pub fn create_shared_anno_file(&mut self, id: &str, name: &str, data: Vec<u8>) -> Result<(), io::Error> {
        if self.doc.dir.get_shared_anno_file().is_some() {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "Shared annotation file already exists."));
        }

        // 1. Insert the shared annotation file itself
        let anno_file = FileRecord::new(id.to_string(), name.to_string(), String::new(), FileType::SharedAnno);
        self.doc.insert_file(anno_file, DataPool::new(data));
        
        // 2. Add an INCL chunk to every page
        let page_ids: Vec<String> = self.doc.dir.get_all_page_ids();
        for page_id in page_ids {
            let mut page_data = self.doc.data.get_mut(&page_id).unwrap();
            
            let mut new_data = Vec::new();
            let mut writer = IffWriter::new(Cursor::new(&mut new_data));
            let mut reader = IffReader::new(Cursor::new(&page_data.data));
            
            let mut info_chunk_found = false;

            // Copy existing chunks
            while let Some(chunk) = reader.next_chunk()? {
                writer.put_chunk(&chunk.id)?;
                writer.write_all(reader.get_chunk_data(&chunk)?)?;
                writer.close_chunk()?;
                if chunk.id == "INFO" {
                    // Add the INCL chunk right after the INFO chunk
                    writer.put_chunk("INCL")?;
                    writer.write_all(id.as_bytes())?;
                    writer.close_chunk()?;
                    info_chunk_found = true;
                }
            }

            // If no INFO chunk, add INCL at the end (inside the FORM)
            if !info_chunk_found {
                // This is a simplification. A robust implementation would need to
                // re-wrap the entire FORM chunk.
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Cannot add shared annotations to a page without an INFO chunk (simplification)."));
            }

            page_data.data = new_data;
        }

        Ok(())
    }

    /// Helper to move a page by its ID.
    fn move_page_by_id(&mut self, id: &str, to_page_num: i32) -> Result<(), io::Error> {
        self.doc.dir.move_file_to_page_pos(id, to_page_num)
    }
    
    /// Helper to parse a byte slice for `INCL` chunk IDs.
    fn parse_included_ids(&self, data: &[u8]) -> Result<Vec<String>, io::Error> {
        let mut ids = Vec::new();
        let mut reader = IffReader::new(Cursor::new(data));
        if let Some(form_chunk) = reader.next_chunk()? {
            if !form_chunk.id.starts_with("FORM") {
                return Ok(ids); // Not a valid DjVu page file
            }
            // Get data within the FORM chunk
            let form_data = reader.get_chunk_data(&form_chunk)?;
            let mut inner_reader = IffReader::new(Cursor::new(form_data));
            while let Some(chunk) = inner_reader.next_chunk()? {
                if chunk.id == "INCL" {
                    let incl_data = inner_reader.get_chunk_data(&chunk)?;
                    ids.push(String::from_utf8_lossy(incl_data).trim().to_string());
                }
            }
        }
        Ok(ids)
    }

    /// Helper to build a map of file relationships (parent->children and child->parents).
    fn build_ref_map(&self) -> Result<RefMap, io::Error> {
        let mut parents: HashMap<String, HashSet<String>> = HashMap::new();
        let mut children: HashMap<String, HashSet<String>> = HashMap::new();

        for file in &self.doc.dir.files {
            let child_ids = self.parse_included_ids(&self.doc.data[&file.id].data)?;
            for child_id in child_ids {
                // file -> child
                children.entry(file.id.clone()).or_default().insert(child_id.clone());
                // child -> parent
                parents.entry(child_id).or_default().insert(file.id.clone());
            }
        }
        Ok(RefMap { parents, children })
    }
}

/// A map of file reference relationships.
struct RefMap {
    /// Maps a child ID to a set of its parent IDs.
    parents: HashMap<String, HashSet<String>>,
    /// Maps a parent ID to a set of its child IDs.
    children: HashMap<String, HashSet<String>>,
}