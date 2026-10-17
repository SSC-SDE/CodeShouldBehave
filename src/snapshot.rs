use crate::{
    core::Cancellation,
    git::{BatchBlob, batch_blobs},
};
use std::path::Path;

/// Bounds for one resumable repository-reader unit. Completed chunks are
/// independently cacheable, so a later attempt does not need to reread them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReadLimits {
    pub objects_per_chunk: usize,
    pub bytes_per_object: usize,
    pub bytes_per_chunk: usize,
}

impl Default for ReadLimits {
    fn default() -> Self {
        Self {
            objects_per_chunk: 128,
            bytes_per_object: 512 * 1024,
            bytes_per_chunk: 64 * 1024 * 1024,
        }
    }
}

pub(crate) fn read_chunk(
    repository: &Path,
    object_ids: &[String],
    limits: ReadLimits,
    cancellation: &Cancellation,
) -> Result<Vec<BatchBlob>, String> {
    if object_ids.len() > limits.objects_per_chunk {
        return Err(format!(
            "Snapshot reader chunk contains {} objects; limit is {}",
            object_ids.len(),
            limits.objects_per_chunk
        ));
    }
    batch_blobs(
        repository,
        object_ids,
        limits.bytes_per_object,
        limits.bytes_per_chunk,
        cancellation,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_an_unbounded_chunk_before_git_access() {
        let limits = ReadLimits {
            objects_per_chunk: 1,
            ..ReadLimits::default()
        };
        let ids = vec!["a".repeat(40), "b".repeat(40)];
        let error = read_chunk(
            Path::new("does-not-exist"),
            &ids,
            limits,
            &Cancellation::default(),
        )
        .unwrap_err();
        assert!(error.contains("limit is 1"));
    }
}
