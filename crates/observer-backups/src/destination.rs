//! Where a backup's copies go: a folder of the host, an SFTP server, an S3-compatible store. Each
//! takes the finished file, puts it under its final name only once it is whole, deletes what
//! Observer wrote there and nothing else, and reads a copy back for a download.

pub mod local;
pub mod s3;
pub mod sftp;
pub mod sigv4;
