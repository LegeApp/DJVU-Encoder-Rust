2. Advice and Refinements for Your Existing Code

Your existing DjVuDocument.rs and DjVmDir.rs code is a solid foundation. Here are some suggestions to make it more robust, idiomatic, and aligned with the new editor.
In document.rs (Your DjVuDocument.rs):

    Integrate DjVmNav and Bookmark:
    Your DjVmNav struct should use the new Bookmark struct from nav.rs to support nested bookmarks.
    Generated rust

          
    // In document.rs, near the top
    use crate::nav::{DjVmNav, Bookmark}; 

    // ... in the DjVuDocument struct
    pub struct DjVuDocument {
        pub dir: DjVmDir,
        pub data: HashMap<String, DataPool>,
        pub nav: Option<DjVmNav>,
    }

    // ... in the DjVuDocument impl
    impl DjVuDocument {
        pub fn set_bookmarks(&mut self, bookmarks: Vec<Bookmark>) {
            self.nav = Some(DjVmNav { bookmarks });
        }
    }

        

    IGNORE_WHEN_COPYING_START

Use code with caution. Rust
IGNORE_WHEN_COPYING_END

Improve FileType Enum:
Your enum is missing SharedAnno and Thumbnails. Add them to be feature-complete with the C++ DjVmDir. Even if you don't generate thumbnails, you need to be able to store them.
Generated rust

      
// In document.rs
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
#[repr(u8)]
pub enum FileType {
    Include = 0,
    Page = 1,
    Thumbnails = 2,
    SharedAnno = 3,
}

    

IGNORE_WHEN_COPYING_START
Use code with caution. Rust
IGNORE_WHEN_COPYING_END

Using #[repr(u8)] makes conversions to and from integers explicit and safe.

Refine DjVmDir and FileRecord:
The DjVmDir struct inside document.rs should be more than just a Vec<FileRecord>. It needs efficient lookups by ID and page number, and methods to manage page ordering.
Generated rust

      
// In document.rs
#[derive(Debug, Clone, Default)]
pub struct DjVmDir {
    pub files: Vec<FileRecord>, // The authoritative order
    id_map: HashMap<String, usize>, // Maps ID to index in `files`
    page_map: Vec<usize>, // Maps page number to index in `files`
}

impl DjVmDir {
    pub fn new() -> Self { Self::default() }

    // ... add methods like insert_file, move_file_to_page_pos, etc.
    // These methods will update all three fields (`files`, `id_map`, `page_map`)
    // to keep them in sync.
}

    

IGNORE_WHEN_COPYING_START
Use code with caution. Rust
IGNORE_WHEN_COPYING_END

This structure provides both ordered iteration (files) and fast lookups (id_map, page_map).

Correct write_bundled Logic:
The IFF format requires that chunks are padded to an even number of bytes. Your current write_bundled logic handles this for file data but not for the DIRM and NAVM chunks.
Generated rust

      
// In DjVuDocument::write_bundled
// ...
iff.put_chunk("DIRM")?;
iff.write_all(&real_dirm_buffer)?;
iff.close_chunk()?; // This needs to handle padding

// In IffWriter::close_chunk
pub fn close_chunk(&mut self) -> Result<(), io::Error> {
    let mut current_pos = self.writer.stream_position()?;
    let start_pos = self.stack.pop().unwrap();
    let size = (current_pos - start_pos - 8) as u32; // -8 for ID and size field
    
    // Add padding if size is odd
    if size % 2 != 0 {
        self.writer.write_all(&[0])?;
        current_pos += 1;
    }

    self.writer.seek(SeekFrom::Start(start_pos + 4))?;
    self.writer.write_all(&size.to_be_bytes())?;
    self.writer.seek(SeekFrom::Start(current_pos))?;
    Ok(())
}

    

IGNORE_WHEN_COPYING_START

    Use code with caution. Rust
    IGNORE_WHEN_COPYING_END

    Separate document.rs:
    Your initial file seems to combine the definitions for DjVuDocument, DjVmDir, FileRecord, IffWriter, etc. For better organization, consider splitting them:

        src/document.rs: DjVuDocument, DataPool.

        src/dir.rs: DjVmDir, FileRecord, FileType.

        src/iff.rs: IffReader, IffWriter, IffChunk.

        src/nav.rs: DjVmNav, Bookmark.

        src/doc_editor.rs: DjVuDocEditor.
        Then, use a lib.rs or mod.rs to declare them as public modules.

Regarding DjVmDir0.rs:

Your implementation of DjVmDir0 is fine. It correctly captures the structure of the legacy DIR0 chunk. Since your goal is a modern encoder, you will likely not need to write this format, but having the structure defined is harmless.
Summary of the Refactored Structure

    DjVuDocEditor is your main entry point for building a document. You create an editor, call methods like insert_page, remove_page, and set_bookmarks, and finally call .build() to get a finalized DjVuDocument.

    The resulting DjVuDocument is a read-only representation of the complete document. It holds all the data and metadata.

    The DjVuDocument can then be serialized into either bundled or indirect format using its write_bundled or write_indirect methods.

    The DjVmNav struct now correctly encodes bookmarks into the standard S-expression format, supporting nested outlines.

This design provides a clean separation between the mutable "editing" phase and the immutable "finished document" phase, which is a very common and effective pattern in Rust.