use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use rustix::fs::{CWD, RenameFlags, renameat_with};

use super::{Destination, Error, ErrorCategory, Result};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A cheap cancellation signal shared by process handling and synchronous work.
#[derive(Clone, Debug, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    /// Request cancellation. Work observes the request at write and commit boundaries.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Return whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub(crate) fn signal_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.0)
    }
}

/// The successful terminal state of an output transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputOutcome {
    /// All bytes were flushed and, for files, atomically committed.
    Complete,
    /// A downstream stdout consumer closed the pipe normally.
    DownstreamClosed,
}

/// Write one complete output transaction to a validated destination.
///
/// File output remains temporary until `produce` succeeds, the file flushes and
/// syncs, and cancellation is checked for the final time. Any earlier return
/// drops and removes the sibling temporary file.
pub fn write_output<F>(
    destination: &Destination,
    cancellation: &Cancellation,
    produce: F,
) -> Result<OutputOutcome>
where
    F: FnOnce(&mut dyn Write) -> io::Result<()>,
{
    match destination.file_path() {
        Some(path) => write_file(path, destination.force(), cancellation, produce),
        None => {
            let stdout = io::stdout();
            let mut stdout = stdout.lock();
            write_stream(&mut stdout, cancellation, produce)
        }
    }
}

/// Write binary-safe data to a stream without adding presentation output.
///
/// This generic seam lets callers and tests supply stdout-like writers while
/// keeping broken-pipe and cancellation policy in the core.
pub fn write_stream<W, F>(
    mut writer: W,
    cancellation: &Cancellation,
    produce: F,
) -> Result<OutputOutcome>
where
    W: Write,
    F: FnOnce(&mut dyn Write) -> io::Result<()>,
{
    check_cancelled(cancellation)?;
    let mut writer = CancellableWriter::new(&mut writer, cancellation);

    let production = produce(&mut writer).and_then(|()| writer.flush());
    if cancellation.is_cancelled() {
        return Err(interrupted());
    }

    match production {
        Ok(()) => Ok(OutputOutcome::Complete),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {
            Ok(OutputOutcome::DownstreamClosed)
        }
        Err(error) => Err(io_error("write output stream", error)),
    }
}

fn write_file<F>(
    destination: &Path,
    force: bool,
    cancellation: &Cancellation,
    produce: F,
) -> Result<OutputOutcome>
where
    F: FnOnce(&mut dyn Write) -> io::Result<()>,
{
    check_cancelled(cancellation)?;
    refuse_existing(destination, force)?;

    let mut temporary = TemporaryFile::create_sibling(destination)?;
    let production = {
        let mut writer = CancellableWriter::new(&mut temporary.file, cancellation);
        produce(&mut writer).and_then(|()| writer.flush())
    };

    if cancellation.is_cancelled() {
        return Err(interrupted());
    }
    production.map_err(|error| io_error("write temporary output", error))?;
    temporary
        .file
        .sync_all()
        .map_err(|error| io_error("sync temporary output", error))?;
    check_cancelled(cancellation)?;

    temporary.commit(destination, force)?;
    Ok(OutputOutcome::Complete)
}

struct CancellableWriter<'a, W> {
    writer: &'a mut W,
    cancellation: &'a Cancellation,
}

impl<'a, W> CancellableWriter<'a, W> {
    fn new(writer: &'a mut W, cancellation: &'a Cancellation) -> Self {
        Self {
            writer,
            cancellation,
        }
    }
}

impl<W: Write> Write for CancellableWriter<'_, W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.cancellation.is_cancelled() {
            return Err(io::Error::other("operation interrupted"));
        }
        self.writer.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.cancellation.is_cancelled() {
            return Err(io::Error::other("operation interrupted"));
        }
        self.writer.flush()
    }
}

fn refuse_existing(destination: &Path, force: bool) -> Result<()> {
    if force {
        return Ok(());
    }

    match fs::symlink_metadata(destination) {
        Ok(_) => Err(existing_destination(destination)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("inspect output destination", error)),
    }
}

struct TemporaryFile {
    file: File,
    path: Option<PathBuf>,
}

impl TemporaryFile {
    fn create_sibling(destination: &Path) -> Result<Self> {
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        let file_name = destination.file_name().ok_or_else(|| {
            Error::new(
                ErrorCategory::Io,
                format!(
                    "output destination '{}' has no file name",
                    destination.display()
                ),
            )
        })?;

        for _ in 0..128 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let mut temporary_name = OsString::from(".");
            temporary_name.push(file_name);
            temporary_name.push(format!(".pcx.tmp.{}.{sequence}", std::process::id()));
            let path = parent.join(temporary_name);

            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        file,
                        path: Some(path),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error("create sibling temporary output", error)),
            }
        }

        Err(Error::new(
            ErrorCategory::Io,
            format!(
                "could not create a unique temporary output beside '{}'",
                destination.display()
            ),
        ))
    }

    fn commit(&mut self, destination: &Path, force: bool) -> Result<()> {
        let temporary = self.path.as_ref().expect("uncommitted temporary path");
        let commit = if force {
            fs::rename(temporary, destination)
                .map_err(|error| io_error("atomically replace output destination", error))
        } else {
            renameat_with(CWD, temporary, CWD, destination, RenameFlags::NOREPLACE).map_err(
                |error| {
                    if error == rustix::io::Errno::EXIST {
                        existing_destination(destination)
                    } else {
                        io_error("atomically commit output destination", error.into())
                    }
                },
            )
        };

        if commit.is_ok() {
            self.path = None;
        }
        commit
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = fs::remove_file(path);
        }
    }
}

fn check_cancelled(cancellation: &Cancellation) -> Result<()> {
    if cancellation.is_cancelled() {
        Err(interrupted())
    } else {
        Ok(())
    }
}

fn interrupted() -> Error {
    Error::new(ErrorCategory::Interrupted, "operation interrupted")
}

fn existing_destination(destination: &Path) -> Error {
    Error::new(
        ErrorCategory::Usage,
        format!(
            "output destination '{}' already exists; pass --force to replace it",
            destination.display()
        ),
    )
}

fn io_error(action: &str, error: io::Error) -> Error {
    Error::new(ErrorCategory::Io, format!("{action}: {error}"))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{self, Write},
        path::{Path, PathBuf},
    };

    use super::{Cancellation, OutputOutcome, write_output, write_stream};
    use crate::core::{Destination, ErrorCategory};

    fn test_directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "pcx-sink-{name}-{}-{}",
            std::process::id(),
            super::TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create test directory");
        path
    }

    fn assert_no_temporary_files(directory: &Path) {
        let entries = fs::read_dir(directory)
            .expect("read test directory")
            .collect::<io::Result<Vec<_>>>()
            .expect("read entries");
        assert!(
            entries
                .iter()
                .all(|entry| !entry.file_name().to_string_lossy().contains(".pcx.tmp.")),
            "temporary output remained in {directory:?}"
        );
    }

    #[test]
    fn commits_complete_file_with_a_sibling_atomic_rename() {
        let directory = test_directory("commit");
        let path = directory.join("frame.pcd");
        let destination = Destination::file(&path, false).expect("destination");

        let outcome = write_output(&destination, &Cancellation::default(), |writer| {
            writer.write_all(b"complete binary\0payload")
        })
        .expect("output should commit");

        assert_eq!(outcome, OutputOutcome::Complete);
        assert_eq!(
            fs::read(&path).expect("committed output"),
            b"complete binary\0payload"
        );
        assert_no_temporary_files(&directory);
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn failure_injection_never_commits_partial_output() {
        let directory = test_directory("failure");
        let path = directory.join("frame.pcd");
        let destination = Destination::file(&path, false).expect("destination");

        let error = write_output(&destination, &Cancellation::default(), |writer| {
            writer.write_all(b"partial")?;
            Err(io::Error::other("injected encoder failure"))
        })
        .expect_err("production must fail");

        assert_eq!(error.category(), ErrorCategory::Io);
        assert!(!path.exists());
        assert_no_temporary_files(&directory);
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn refuses_existing_destination_without_force_and_replaces_with_force() {
        let directory = test_directory("force");
        let path = directory.join("frame.pcd");
        fs::write(&path, b"original").expect("seed destination");

        let refusal = write_output(
            &Destination::file(&path, false).expect("destination"),
            &Cancellation::default(),
            |writer| writer.write_all(b"replacement"),
        )
        .expect_err("replacement must require force");
        assert_eq!(refusal.category(), ErrorCategory::Usage);
        assert_eq!(fs::read(&path).expect("original output"), b"original");

        write_output(
            &Destination::file(&path, true).expect("destination"),
            &Cancellation::default(),
            |writer| writer.write_all(b"replacement"),
        )
        .expect("forced replacement should commit");
        assert_eq!(fs::read(&path).expect("replacement output"), b"replacement");
        assert_no_temporary_files(&directory);
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn cancellation_during_production_removes_the_temporary_output() {
        let directory = test_directory("cancel");
        let path = directory.join("frame.pcd");
        let destination = Destination::file(&path, false).expect("destination");
        let cancellation = Cancellation::default();

        let error = write_output(&destination, &cancellation, |writer| {
            writer.write_all(b"partial")?;
            cancellation.cancel();
            Ok(())
        })
        .expect_err("cancellation must fail");

        assert_eq!(error.category(), ErrorCategory::Interrupted);
        assert!(!path.exists());
        assert_no_temporary_files(&directory);
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    #[test]
    fn no_clobber_commit_rejects_a_destination_created_during_production() {
        let directory = test_directory("race");
        let path = directory.join("frame.pcd");
        let destination = Destination::file(&path, false).expect("destination");

        let error = write_output(&destination, &Cancellation::default(), |writer| {
            writer.write_all(b"pcx output")?;
            fs::write(&path, b"racing output")?;
            Ok(())
        })
        .expect_err("racing destination must not be replaced");

        assert_eq!(error.category(), ErrorCategory::Usage);
        assert_eq!(fs::read(&path).expect("racing output"), b"racing output");
        assert_no_temporary_files(&directory);
        fs::remove_dir_all(directory).expect("remove test directory");
    }

    struct BrokenPipe;

    impl Write for BrokenPipe {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "downstream closed",
            ))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn binary_stream_is_unchanged_and_broken_pipe_is_normal() {
        let payload = b"\0\xffdiagnostic-looking bytes\n";
        let mut output = Vec::new();
        let complete = write_stream(&mut output, &Cancellation::default(), |writer| {
            writer.write_all(payload)
        })
        .expect("binary stream should succeed");
        assert_eq!(complete, OutputOutcome::Complete);
        assert_eq!(output, payload);

        let closed = write_stream(BrokenPipe, &Cancellation::default(), |writer| {
            writer.write_all(payload)
        })
        .expect("broken pipe is expected termination");
        assert_eq!(closed, OutputOutcome::DownstreamClosed);
    }
}
