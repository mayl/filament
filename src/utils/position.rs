//! Tracking of source positions
use codespan_reporting::files::SimpleFiles;
use itertools::Itertools;
use std::{cmp, fmt::Write, mem, sync};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
/// Handle to a position in a [PositionTable]
/// The index refers to the index in the [PositionTable::indices] vector.
pub struct PosIdx(u32);

#[derive(Clone, Copy, PartialEq, Eq)]
/// Handle to a file in a [PositionTable]
/// The index refers to the index in the [PositionTable::files] vector.
pub struct FileIdx(usize);

impl FileIdx {
    /// Get the index of the file
    pub fn get(self) -> usize {
        self.0
    }
}

/// The data associated with a position
pub struct PosData {
    /// The file in the program. The index refers to the index in the
    /// [PositionTable::files] vector.
    pub file: FileIdx,
    /// Start of the span
    pub start: usize,
    /// End of the span
    pub end: usize,
}

/// Source position information for a full program
pub struct PositionTable {
    /// The source files of the program
    files: SimpleFiles<String, String>,
    /// Mapping from indexes to position data
    indices: Vec<PosData>,
}

impl Default for PositionTable {
    fn default() -> Self {
        Self::new()
    }
}

impl PositionTable {
    /// The unknown position
    pub const UNKNOWN: PosIdx = PosIdx(0);

    /// Create a new position table where the first file and first position are unknown
    pub fn new() -> Self {
        let mut table = PositionTable {
            files: SimpleFiles::new(),
            indices: Vec::new(),
        };
        table.add_file("unknown".to_string(), "".to_string());
        let pos = table.add_pos(FileIdx(0), 0, 0);
        debug_assert!(pos == Self::UNKNOWN);
        table
    }

    /// Return handle to the files in the position table
    pub fn files(&self) -> &SimpleFiles<String, String> {
        &self.files
    }

    /// Add a new file to the position table
    pub fn add_file(&mut self, name: String, source: String) -> FileIdx {
        let idx = self.files.add(name, source);
        FileIdx(idx)
    }

    pub fn get_file_info(&self, pos: PosIdx) -> (&str, &str) {
        let pos_d = self.get_pos(pos);
        self.get_file_data(pos_d.file)
    }

    /// Add a new position to the position table
    pub fn add_pos(
        &mut self,
        file: FileIdx,
        start: usize,
        end: usize,
    ) -> PosIdx {
        let pos = PosData { file, start, end };
        let pos_idx = self.indices.len();
        self.indices.push(pos);
        PosIdx(pos_idx as u32)
    }

    pub fn get_pos(&self, pos: PosIdx) -> &PosData {
        &self.indices[pos.0 as usize]
    }

    /// Return the name and source of the file
    pub fn get_file_data(&self, file: FileIdx) -> (&str, &str) {
        let file = &self.files.get(file.0).unwrap();
        (file.name(), file.source())
    }
}

/// The global position table
pub struct GlobalPositionTable;

impl GlobalPositionTable {
    /// Return reference to a global [PositionTable]
    pub fn as_mut() -> &'static mut PositionTable {
        static mut SINGLETON: mem::MaybeUninit<PositionTable> =
            mem::MaybeUninit::uninit();
        static ONCE: sync::Once = sync::Once::new();

        // SAFETY:
        // - writing to the singleton is OK because we only do it one time
        // - the ONCE guarantees that SINGLETON is init'ed before assume_init_ref
        unsafe {
            ONCE.call_once(|| {
                SINGLETON.write(PositionTable::new());
                assert!(PositionTable::UNKNOWN == GPosIdx::UNKNOWN.0)
            });
            SINGLETON.assume_init_mut()
        }
    }

    /// Return an immutable reference to the global position table
    pub fn as_ref() -> &'static PositionTable {
        Self::as_mut()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
/// A position index backed by a global [PositionTable]
pub struct GPosIdx(pub PosIdx);

impl GPosIdx {
    /// Symbol for the unknown position.
    /// All [PositionTable] must allocate the first index for this use.
    pub const UNKNOWN: GPosIdx = GPosIdx(PosIdx(0));

    /// Convert the position into an optional.
    /// Returns `None` if the position is the unknown position.
    pub fn into_option(self) -> Option<Self> {
        if self == Self::UNKNOWN {
            None
        } else {
            Some(self)
        }
    }

    /// Returns the
    /// 1. lines associated with this span
    /// 2. start position of the first line in span
    /// 3. line number of the span
    fn get_lines(&self) -> (Vec<&str>, usize, usize) {
        let table = GlobalPositionTable::as_ref();
        let pos_d = table.get_pos(self.0);
        let (_, file) = &table.get_file_data(pos_d.file);

        let lines = file.split('\n').collect_vec();
        let mut pos: usize = 0;
        let mut linum: usize = 1;
        let mut collect_lines = false;
        let mut buf = Vec::new();

        let mut out_line: usize = 0;
        let mut out_idx: usize = 0;
        for l in lines {
            let next_pos = pos + l.len();
            if pos_d.start >= pos && pos_d.start <= next_pos {
                out_line = linum;
                out_idx = pos;
                collect_lines = true;
            }
            if collect_lines && pos_d.end >= pos {
                buf.push(l)
            }
            if pos_d.end <= next_pos {
                break;
            }
            pos = next_pos + 1;
            linum += 1;
        }
        (buf, out_idx, out_line)
    }

    /// Format this position with a the error message `err_msg`
    pub fn format<S: AsRef<str>>(&self, err_msg: S) -> String {
        let table = GlobalPositionTable::as_ref();
        let pos_d = table.get_pos(self.0);
        let (name, _) = &table.get_file_data(pos_d.file);

        let (lines, pos, linum) = self.get_lines();
        let mut buf = name.to_string();

        let l = lines[0];
        let linum_text = format!("{} ", linum);
        let linum_space: String = " ".repeat(linum_text.len());
        let mark: String = "^".repeat(cmp::min(
            pos_d.end - pos_d.start,
            l.len() - (pos_d.start - pos),
        ));
        let space: String = " ".repeat(pos_d.start - pos);
        writeln!(buf).unwrap();
        writeln!(buf, "{}|{}", linum_text, l).unwrap();
        write!(
            buf,
            "{}|{}{} {}",
            linum_space,
            space,
            mark,
            err_msg.as_ref()
        )
        .unwrap();
        buf
    }

    /// Visualizes the span without any message or mkaring
    pub fn show(&self) -> String {
        let (lines, _, linum) = self.get_lines();
        let l = lines[0];
        let linum_text = format!("{} ", linum);
        format!("{}|{}\n", linum_text, l)
    }
}

/// An IR node that may contain position information.
pub trait WithPos {
    /// Copy the span associated with this node.
    fn copy_span(&self) -> GPosIdx;
}
