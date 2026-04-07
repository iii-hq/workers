use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use walkdir::WalkDir;

use crate::config::SensorConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetrics {
    pub path: String,
    pub language: String,
    pub line_count: usize,
    pub code_lines: usize,
    pub complexity: usize,
    pub max_depth: usize,
    pub function_count: usize,
    pub avg_function_length: f64,
    pub import_count: usize,
    pub duplicate_lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageSummary {
    pub files: usize,
    pub lines: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanSummary {
    pub total_files: usize,
    pub total_lines: usize,
    pub avg_complexity: f64,
    pub languages: HashMap<String, LanguageSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    pub files: Vec<FileMetrics>,
    pub summary: ScanSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionScores {
    pub complexity: f64,
    pub coupling: f64,
    pub cohesion: f64,
    pub size: f64,
    pub duplication: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreResult {
    pub score: f64,
    pub dimensions: DimensionScores,
    pub grade: String,
    pub file_count: usize,
    pub timestamp: String,
}

pub fn detect_language(ext: &str) -> &str {
    match ext {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "py" => "python",
        "go" => "go",
        _ => "unknown",
    }
}

pub fn count_complexity(content: &str, ext: &str) -> usize {
    let keywords: &[&str] = match ext {
        "rs" => &[
            "if ", "else ", "match ", "for ", "while ", "loop ", "?", ".unwrap(",
        ],
        "ts" | "tsx" | "js" | "jsx" => &[
            "if ", "else ", "switch ", "for ", "while ", "try ", "catch ", "? ",
        ],
        "py" => &[
            "if ", "elif ", "else:", "for ", "while ", "try:", "except ", "with ",
        ],
        "go" => &["if ", "else ", "switch ", "for ", "select ", "case "],
        _ => &["if ", "else ", "for ", "while "],
    };
    content
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            keywords.iter().filter(|kw| trimmed.contains(**kw)).count()
        })
        .sum()
}

pub fn count_max_depth(content: &str) -> usize {
    let mut max_depth: usize = 0;
    let mut current_depth: usize = 0;
    for line in content.lines() {
        for ch in line.chars() {
            if ch == '{' {
                current_depth += 1;
                if current_depth > max_depth {
                    max_depth = current_depth;
                }
            } else if ch == '}' {
                current_depth = current_depth.saturating_sub(1);
            }
        }
    }
    max_depth
}

pub fn count_functions(content: &str, ext: &str) -> usize {
    let patterns: &[&str] = match ext {
        "rs" => &["fn "],
        "ts" | "tsx" | "js" | "jsx" => &["function ", "=> {", "=> ("],
        "py" => &["def "],
        "go" => &["func "],
        _ => &["fn ", "def ", "function ", "func "],
    };
    content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            patterns.iter().any(|p| trimmed.contains(p))
        })
        .count()
}

pub fn count_imports(content: &str, ext: &str) -> usize {
    content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            match ext {
                "rs" => trimmed.starts_with("use "),
                "ts" | "tsx" | "js" | "jsx" => {
                    trimmed.starts_with("import ") || trimmed.starts_with("require(")
                }
                "py" => trimmed.starts_with("import ") || trimmed.starts_with("from "),
                "go" => trimmed.starts_with("import ") || trimmed.starts_with("\""),
                _ => false,
            }
        })
        .count()
}

pub fn count_code_lines(content: &str) -> usize {
    content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty()
                && !trimmed.starts_with("//")
                && !trimmed.starts_with('#')
                && !trimmed.starts_with("/*")
                && !trimmed.starts_with('*')
                && !trimmed.starts_with("*/")
        })
        .count()
}

pub fn count_duplicate_lines(content: &str) -> usize {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.len() >= 10 {
            *seen.entry(trimmed).or_insert(0) += 1;
        }
    }
    seen.values().filter(|&&count| count > 1).map(|c| c - 1).sum()
}

pub fn analyze_file(path: &Path, ext: &str) -> Option<FileMetrics> {
    let content = std::fs::read_to_string(path).ok()?;
    let line_count = content.lines().count();
    let code_lines = count_code_lines(&content);
    let complexity = count_complexity(&content, ext);
    let max_depth = count_max_depth(&content);
    let function_count = count_functions(&content, ext);
    let avg_function_length = if function_count > 0 {
        code_lines as f64 / function_count as f64
    } else {
        code_lines as f64
    };
    let import_count = count_imports(&content, ext);
    let duplicate_lines = count_duplicate_lines(&content);

    Some(FileMetrics {
        path: path.to_string_lossy().to_string(),
        language: detect_language(ext).to_string(),
        line_count,
        code_lines,
        complexity,
        max_depth,
        function_count,
        avg_function_length,
        import_count,
        duplicate_lines,
    })
}

pub fn scan_directory(dir: &str, extensions: &[String], max_file_size_kb: u64) -> ScanResult {
    let max_bytes = max_file_size_kb * 1024;
    let mut files = Vec::new();
    let mut languages: HashMap<String, LanguageSummary> = HashMap::new();

    for entry in WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e.to_string(),
            None => continue,
        };

        if !extensions.iter().any(|allowed| allowed == &ext) {
            continue;
        }

        if let Ok(metadata) = std::fs::metadata(path) {
            if metadata.len() > max_bytes {
                continue;
            }
        }

        if let Some(metrics) = analyze_file(path, &ext) {
            let lang_entry = languages
                .entry(metrics.language.clone())
                .or_insert(LanguageSummary { files: 0, lines: 0 });
            lang_entry.files += 1;
            lang_entry.lines += metrics.line_count;
            files.push(metrics);
        }
    }

    let total_files = files.len();
    let total_lines: usize = files.iter().map(|f| f.line_count).sum();
    let avg_complexity = if total_files > 0 {
        files.iter().map(|f| f.complexity as f64).sum::<f64>() / total_files as f64
    } else {
        0.0
    };

    ScanResult {
        files,
        summary: ScanSummary {
            total_files,
            total_lines,
            avg_complexity,
            languages,
        },
    }
}

pub fn geometric_mean(scores: &[f64]) -> f64 {
    if scores.is_empty() || scores.iter().any(|s| *s <= 0.0) {
        return 0.0;
    }
    let ln_sum: f64 = scores.iter().map(|s| s.ln()).sum::<f64>() / scores.len() as f64;
    ln_sum.exp()
}

pub fn compute_score(scan: &ScanResult, config: &SensorConfig) -> ScoreResult {
    let files = &scan.files;
    let total = files.len().max(1) as f64;

    let avg_complexity_per_100 = if total > 0.0 {
        files
            .iter()
            .map(|f| {
                if f.code_lines > 0 {
                    (f.complexity as f64 / f.code_lines as f64) * 100.0
                } else {
                    0.0
                }
            })
            .sum::<f64>()
            / total
    } else {
        0.0
    };
    let complexity_score = (100.0 - (avg_complexity_per_100 - 5.0).max(0.0) * 5.0).clamp(0.0, 100.0);

    let avg_imports = files.iter().map(|f| f.import_count as f64).sum::<f64>() / total;
    let coupling_score = (100.0 - (avg_imports - 5.0).max(0.0) * 3.0).clamp(0.0, 100.0);

    let cohesion_raw: f64 = files
        .iter()
        .map(|f| {
            if f.function_count > 0 {
                let ideal = (f.code_lines as f64 / 20.0).max(1.0);
                let ratio = f.function_count as f64 / ideal;
                (1.0 - (ratio - 1.0).abs()).max(0.0) * 100.0
            } else {
                50.0
            }
        })
        .sum::<f64>()
        / total;
    let cohesion_score = cohesion_raw.clamp(0.0, 100.0);

    let avg_lines = files.iter().map(|f| f.code_lines as f64).sum::<f64>() / total;
    let size_score = (100.0 - (avg_lines - 100.0).max(0.0) * 0.2).clamp(0.0, 100.0);

    let avg_dup_ratio = files
        .iter()
        .map(|f| {
            if f.code_lines > 0 {
                f.duplicate_lines as f64 / f.code_lines as f64
            } else {
                0.0
            }
        })
        .sum::<f64>()
        / total;
    let duplication_score = (100.0 - avg_dup_ratio * 200.0).clamp(0.0, 100.0);

    let weighted_scores = vec![
        complexity_score,
        coupling_score,
        cohesion_score,
        size_score,
        duplication_score,
    ];

    let weights = [
        config.score_weights.complexity,
        config.score_weights.coupling,
        config.score_weights.cohesion,
        config.score_weights.size,
        config.score_weights.duplication,
    ];

    let weighted: Vec<f64> = weighted_scores
        .iter()
        .zip(weights.iter())
        .map(|(s, w)| (s.max(1.0)) * w)
        .collect();

    let score = geometric_mean(&weighted).clamp(0.0, 100.0);

    let grade = match score as u32 {
        90..=100 => "A",
        80..=89 => "B",
        70..=79 => "C",
        60..=69 => "D",
        _ => "F",
    }
    .to_string();

    let timestamp = chrono::Utc::now().to_rfc3339();

    ScoreResult {
        score,
        dimensions: DimensionScores {
            complexity: complexity_score,
            coupling: coupling_score,
            cohesion: cohesion_score,
            size: size_score,
            duplication: duplication_score,
        },
        grade,
        file_count: files.len(),
        timestamp,
    }
}

pub fn hash_path(path: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_complexity_rust() {
        let code = r#"
fn main() {
    if x > 0 {
        for i in 0..10 {
            while running {
                match cmd {
                    _ => {}
                }
            }
        }
    }
}
"#;
        let c = count_complexity(code, "rs");
        assert!(c >= 4);
    }

    #[test]
    fn test_count_complexity_python() {
        let code = "if x:\n    for i in range(10):\n        while True:\n            pass\n";
        let c = count_complexity(code, "py");
        assert!(c >= 3);
    }

    #[test]
    fn test_count_max_depth() {
        let code = "fn main() {\n    if true {\n        {\n        }\n    }\n}\n";
        assert_eq!(count_max_depth(code), 3);
    }

    #[test]
    fn test_count_functions_rust() {
        let code = "fn main() {}\nfn helper() {}\npub fn public_fn() {}\n";
        assert_eq!(count_functions(code, "rs"), 3);
    }

    #[test]
    fn test_count_functions_python() {
        let code = "def main():\n    pass\ndef helper():\n    pass\n";
        assert_eq!(count_functions(code, "py"), 2);
    }

    #[test]
    fn test_count_imports_rust() {
        let code = "use std::io;\nuse serde::Serialize;\nfn main() {}\n";
        assert_eq!(count_imports(code, "rs"), 2);
    }

    #[test]
    fn test_count_code_lines() {
        let code = "fn main() {\n    // comment\n\n    let x = 1;\n}\n";
        assert_eq!(count_code_lines(code), 3);
    }

    #[test]
    fn test_geometric_mean() {
        let scores = vec![100.0, 100.0, 100.0];
        assert!((geometric_mean(&scores) - 100.0).abs() < 0.01);

        let empty: Vec<f64> = vec![];
        assert_eq!(geometric_mean(&empty), 0.0);

        let with_zero = vec![0.0, 50.0];
        assert_eq!(geometric_mean(&with_zero), 0.0);
    }

    #[test]
    fn test_hash_path_deterministic() {
        let h1 = hash_path("/tmp/test");
        let h2 = hash_path("/tmp/test");
        assert_eq!(h1, h2);

        let h3 = hash_path("/tmp/other");
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_detect_language() {
        assert_eq!(detect_language("rs"), "rust");
        assert_eq!(detect_language("ts"), "typescript");
        assert_eq!(detect_language("py"), "python");
        assert_eq!(detect_language("go"), "go");
        assert_eq!(detect_language("xyz"), "unknown");
    }

    #[test]
    fn test_count_duplicate_lines() {
        let code = "let x = something_long_enough;\nlet x = something_long_enough;\nshort\n";
        assert_eq!(count_duplicate_lines(code), 1);
    }

    #[test]
    fn test_compute_score_empty() {
        let scan = ScanResult {
            files: vec![],
            summary: ScanSummary {
                total_files: 0,
                total_lines: 0,
                avg_complexity: 0.0,
                languages: HashMap::new(),
            },
        };
        let config = SensorConfig::default();
        let result = compute_score(&scan, &config);
        assert!(result.score >= 0.0);
        assert!(!result.grade.is_empty());
    }
}
