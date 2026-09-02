use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{ToolError, ToolResult};

const TOOL: &str = "justrmbg";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Job {
    pub input: PathBuf,
    pub output: PathBuf,
}

#[derive(Debug)]
pub struct JobSet {
    pub jobs: Vec<Job>,
    pub batch_mode: bool,
}

pub fn default_output(input: &Path) -> PathBuf {
    let stem = input
        .file_stem()
        .unwrap_or(input.as_os_str())
        .to_string_lossy();
    input.with_file_name(format!("{stem}-nobg.png"))
}

pub fn prepare(inputs: &[PathBuf], output: Option<&Path>) -> ToolResult<JobSet> {
    for input in inputs {
        if !input.exists() {
            return Err(ToolError::new(
                TOOL,
                format!("input not found: {}", input.display()),
            ));
        }
    }

    let directory_input = inputs.iter().any(|path| path.is_dir());
    let batch_mode = directory_input || inputs.len() > 1;
    if directory_input && inputs.len() != 1 {
        return Err(ToolError::usage(
            TOOL,
            "directory input must be the only input",
        ));
    }
    let output_dir = batch_mode.then_some(output).flatten();
    if directory_input && output_dir.is_none() {
        return Err(ToolError::usage(
            TOOL,
            "directory input requires -o <outputDir>",
        ));
    }
    if let Some(output_dir) = output_dir
        && output_dir.exists()
        && !output_dir.is_dir()
    {
        return Err(ToolError::new(
            TOOL,
            format!("batch output is not a directory: {}", output_dir.display()),
        ));
    }

    let jobs: Vec<Job> = if directory_input {
        let output_dir = output_dir.unwrap();
        let mut files = fs::read_dir(&inputs[0])
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| ToolError::new(TOOL, error.to_string()))?
            .into_iter()
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        files.sort();
        if files.is_empty() {
            return Err(ToolError::new(
                TOOL,
                format!("no files found in {}", inputs[0].display()),
            ));
        }
        files
            .into_iter()
            .map(|input| {
                let stem = input
                    .file_stem()
                    .unwrap_or(input.as_os_str())
                    .to_string_lossy()
                    .into_owned();
                Job {
                    input,
                    output: output_dir.join(format!("{stem}.png")),
                }
            })
            .collect()
    } else {
        inputs
            .iter()
            .map(|input| {
                let output = if let Some(directory) = output_dir {
                    let stem = input
                        .file_stem()
                        .unwrap_or(input.as_os_str())
                        .to_string_lossy();
                    directory.join(format!("{stem}-nobg.png"))
                } else {
                    output
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| default_output(input))
                };
                Job {
                    input: input.clone(),
                    output,
                }
            })
            .collect()
    };

    reject_collisions(&jobs)?;
    Ok(JobSet { jobs, batch_mode })
}

pub fn reject_model_overwrite(jobs: &[Job], model: &Path) -> ToolResult {
    let model_key = path_key(model)?;
    for job in jobs {
        if path_key(&job.output)? == model_key {
            return Err(ToolError::new(
                TOOL,
                format!(
                    "output would overwrite the ONNX model: {}",
                    job.output.display()
                ),
            ));
        }
    }
    Ok(())
}

fn reject_collisions(jobs: &[Job]) -> ToolResult {
    let inputs = jobs
        .iter()
        .map(|job| path_key(&job.input))
        .collect::<ToolResult<HashSet<_>>>()?;
    let mut outputs = HashSet::new();
    for job in jobs {
        let key = path_key(&job.output)?;
        if inputs.contains(&key) {
            return Err(ToolError::new(
                TOOL,
                format!("output would overwrite an input: {}", job.output.display()),
            ));
        }
        if !outputs.insert(key) {
            return Err(ToolError::new(
                TOOL,
                format!("multiple inputs would write {}", job.output.display()),
            ));
        }
    }
    Ok(())
}

fn path_key(path: &Path) -> ToolResult<String> {
    let absolute =
        std::path::absolute(path).map_err(|error| ToolError::new(TOOL, error.to_string()))?;
    let identity = fs::canonicalize(&absolute).unwrap_or_else(|_| {
        let parent = absolute.parent().unwrap_or_else(|| Path::new("."));
        let parent = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
        absolute
            .file_name()
            .map_or(parent.clone(), |name| parent.join(name))
    });
    let key = identity.to_string_lossy();
    Ok(if cfg!(windows) {
        key.to_ascii_lowercase()
    } else {
        key.into_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_default_name() {
        assert_eq!(
            default_output(Path::new("somewhere/photo.jpeg")),
            PathBuf::from("somewhere/photo-nobg.png")
        );
    }

    #[test]
    fn protects_inputs_and_models() {
        let jobs = [Job {
            input: "a.jpg".into(),
            output: "model.onnx".into(),
        }];
        assert!(reject_model_overwrite(&jobs, Path::new("model.onnx")).is_err());
    }

    #[test]
    fn batch_preflight_does_not_create_output_directory() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("input.png"),
            b"not decoded during preflight",
        )
        .unwrap();
        let output = directory.path().join("new/output");
        let jobs = prepare(&[directory.path().to_path_buf()], Some(&output)).unwrap();
        assert!(jobs.batch_mode);
        assert!(!output.exists());
    }

    #[test]
    fn multiple_files_map_output_as_a_directory() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first.jpg");
        let second = directory.path().join("second.png");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();
        let output = directory.path().join("results");

        let jobs = prepare(&[first.clone(), second.clone()], Some(&output)).unwrap();

        assert!(jobs.batch_mode);
        assert_eq!(
            jobs.jobs,
            [
                Job {
                    input: first,
                    output: output.join("first-nobg.png"),
                },
                Job {
                    input: second,
                    output: output.join("second-nobg.png"),
                },
            ]
        );
        assert!(!output.exists());
    }

    #[test]
    fn missing_input_fails_before_output_mapping() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.jpg");
        let output = directory.path().join("results");

        let error = prepare(&[missing], Some(&output)).unwrap_err();

        assert!(error.message().contains("input not found"));
        assert!(!output.exists());
    }
}
