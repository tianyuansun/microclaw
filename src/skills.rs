use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub dir_path: PathBuf,
}

pub struct SkillManager {
    skills_dir: PathBuf,
}

impl SkillManager {
    pub fn new(data_dir: &str) -> Self {
        SkillManager {
            skills_dir: PathBuf::from(data_dir).join("skills"),
        }
    }

    /// Discover all skills by reading subdirectories for SKILL.md files.
    pub fn discover_skills(&self) -> Vec<SkillMetadata> {
        let mut skills = Vec::new();
        let entries = match std::fs::read_dir(&self.skills_dir) {
            Ok(e) => e,
            Err(_) => return skills,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let skill_md = path.join("SKILL.md");
            if !skill_md.exists() {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&skill_md) {
                if let Some((meta, _body)) = parse_skill_md(&content, &path) {
                    skills.push(meta);
                }
            }
        }
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        skills
    }

    /// Load a skill by name, returning metadata and full instruction body.
    pub fn load_skill(&self, name: &str) -> Option<(SkillMetadata, String)> {
        let skills = self.discover_skills();
        for skill in skills {
            if skill.name == name {
                let skill_md = skill.dir_path.join("SKILL.md");
                if let Ok(content) = std::fs::read_to_string(&skill_md) {
                    if let Some((meta, body)) = parse_skill_md(&content, &skill.dir_path) {
                        return Some((meta, body));
                    }
                }
            }
        }
        None
    }

    /// Build a compact skills catalog for the system prompt.
    /// Returns empty string if no skills are available.
    pub fn build_skills_catalog(&self) -> String {
        let skills = self.discover_skills();
        if skills.is_empty() {
            return String::new();
        }
        let mut catalog = String::from("<available_skills>\n");
        for skill in &skills {
            catalog.push_str(&format!("- {}: {}\n", skill.name, skill.description));
        }
        catalog.push_str("</available_skills>");
        catalog
    }

    /// Build a user-facing formatted list of available skills.
    pub fn list_skills_formatted(&self) -> String {
        let skills = self.discover_skills();
        if skills.is_empty() {
            return "No skills available.".into();
        }
        let mut output = format!("Available skills ({}):\n\n", skills.len());
        for skill in &skills {
            output.push_str(&format!("• {} — {}\n", skill.name, skill.description));
        }
        output
    }

    #[allow(dead_code)]
    pub fn skills_dir(&self) -> &PathBuf {
        &self.skills_dir
    }
}

/// Parse a SKILL.md file, extracting frontmatter (name, description) and body.
/// Returns None if the file lacks valid frontmatter with a name field.
fn parse_skill_md(content: &str, dir_path: &std::path::Path) -> Option<(SkillMetadata, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }

    // Find the closing ---
    let after_opening = &trimmed[3..];
    let closing_pos = after_opening.find("\n---")?;
    let frontmatter = &after_opening[..closing_pos];
    let body = after_opening[closing_pos + 4..].trim().to_string();

    let mut name: Option<String> = None;
    let mut description = String::new();

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim().trim_matches('"').trim_matches('\'');
            match key {
                "name" => name = Some(value.to_string()),
                "description" => description = value.to_string(),
                _ => {} // ignore other fields
            }
        }
    }

    let name = name?;

    Some((
        SkillMetadata {
            name,
            description,
            dir_path: dir_path.to_path_buf(),
        },
        body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_skill_md_valid() {
        let content = r#"---
name: pdf
description: Convert documents to PDF
---
Use this skill to convert documents.

## Steps
1. Do the thing
"#;
        let dir = PathBuf::from("/tmp/skills/pdf");
        let result = parse_skill_md(content, &dir);
        assert!(result.is_some());
        let (meta, body) = result.unwrap();
        assert_eq!(meta.name, "pdf");
        assert_eq!(meta.description, "Convert documents to PDF");
        assert_eq!(meta.dir_path, dir);
        assert!(body.contains("Use this skill to convert documents."));
        assert!(body.contains("## Steps"));
    }

    #[test]
    fn test_parse_skill_md_no_frontmatter() {
        let content = "Just some markdown without frontmatter.";
        let dir = PathBuf::from("/tmp/skills/test");
        assert!(parse_skill_md(content, &dir).is_none());
    }

    #[test]
    fn test_parse_skill_md_missing_name() {
        let content = r#"---
description: A skill without a name
---
Body text here.
"#;
        let dir = PathBuf::from("/tmp/skills/test");
        assert!(parse_skill_md(content, &dir).is_none());
    }

    #[test]
    fn test_parse_skill_md_extra_fields_ignored() {
        let content = r#"---
name: data-analysis
description: Analyze datasets
license: MIT
version: 1.0
allowed-tools: bash, python
---
Instructions here.
"#;
        let dir = PathBuf::from("/tmp/skills/data-analysis");
        let result = parse_skill_md(content, &dir);
        assert!(result.is_some());
        let (meta, body) = result.unwrap();
        assert_eq!(meta.name, "data-analysis");
        assert_eq!(meta.description, "Analyze datasets");
        assert!(body.contains("Instructions here."));
    }

    fn test_skills_dir() -> PathBuf {
        std::env::temp_dir().join(format!("microclaw_skills_test_{}", uuid::Uuid::new_v4()))
    }

    fn cleanup(dir: &std::path::Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    fn create_skill(base_dir: &std::path::Path, name: &str, desc: &str, body: &str) {
        let skill_dir = base_dir.join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        let content = format!("---\nname: {name}\ndescription: {desc}\n---\n{body}\n");
        std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn test_discover_skills_empty_dir() {
        let dir = test_skills_dir();
        let sm = SkillManager::new(dir.to_str().unwrap());
        let skills = sm.discover_skills();
        assert!(skills.is_empty());
        cleanup(&dir);
    }

    #[test]
    fn test_discover_skills_with_skills() {
        let dir = test_skills_dir();
        create_skill(&dir, "pdf", "Convert to PDF", "PDF instructions");
        create_skill(&dir, "data-analysis", "Analyze data", "Data instructions");

        let sm = SkillManager::new(dir.to_str().unwrap());
        let skills = sm.discover_skills();
        assert_eq!(skills.len(), 2);
        // Sorted alphabetically
        assert_eq!(skills[0].name, "data-analysis");
        assert_eq!(skills[1].name, "pdf");
        cleanup(&dir);
    }

    #[test]
    fn test_discover_skills_ignores_no_skill_md() {
        let dir = test_skills_dir();
        create_skill(&dir, "valid-skill", "A valid skill", "Instructions");
        // Create a dir without SKILL.md
        let invalid_dir = dir.join("skills").join("no-skill-md");
        std::fs::create_dir_all(&invalid_dir).unwrap();
        std::fs::write(invalid_dir.join("README.md"), "not a skill").unwrap();

        let sm = SkillManager::new(dir.to_str().unwrap());
        let skills = sm.discover_skills();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "valid-skill");
        cleanup(&dir);
    }

    #[test]
    fn test_load_skill_found() {
        let dir = test_skills_dir();
        create_skill(&dir, "pdf", "Convert to PDF", "Use pdflatex to convert.");

        let sm = SkillManager::new(dir.to_str().unwrap());
        let result = sm.load_skill("pdf");
        assert!(result.is_some());
        let (meta, body) = result.unwrap();
        assert_eq!(meta.name, "pdf");
        assert_eq!(meta.description, "Convert to PDF");
        assert!(body.contains("Use pdflatex to convert."));
        cleanup(&dir);
    }

    #[test]
    fn test_load_skill_not_found() {
        let dir = test_skills_dir();
        let sm = SkillManager::new(dir.to_str().unwrap());
        assert!(sm.load_skill("nonexistent").is_none());
        cleanup(&dir);
    }

    #[test]
    fn test_build_skills_catalog_empty() {
        let dir = test_skills_dir();
        let sm = SkillManager::new(dir.to_str().unwrap());
        let catalog = sm.build_skills_catalog();
        assert!(catalog.is_empty());
        cleanup(&dir);
    }

    #[test]
    fn test_build_skills_catalog_with_skills() {
        let dir = test_skills_dir();
        create_skill(&dir, "pdf", "Convert to PDF", "Instructions");
        create_skill(&dir, "data-analysis", "Analyze data", "Instructions");

        let sm = SkillManager::new(dir.to_str().unwrap());
        let catalog = sm.build_skills_catalog();
        assert!(catalog.contains("<available_skills>"));
        assert!(catalog.contains("</available_skills>"));
        assert!(catalog.contains("- pdf: Convert to PDF"));
        assert!(catalog.contains("- data-analysis: Analyze data"));
        cleanup(&dir);
    }

    #[test]
    fn test_list_skills_formatted() {
        let dir = test_skills_dir();
        create_skill(&dir, "pdf", "Convert to PDF", "Instructions");

        let sm = SkillManager::new(dir.to_str().unwrap());
        let formatted = sm.list_skills_formatted();
        assert!(formatted.contains("Available skills (1)"));
        assert!(formatted.contains("• pdf — Convert to PDF"));
        cleanup(&dir);
    }

    #[test]
    fn test_list_skills_formatted_empty() {
        let dir = test_skills_dir();
        let sm = SkillManager::new(dir.to_str().unwrap());
        let formatted = sm.list_skills_formatted();
        assert_eq!(formatted, "No skills available.");
        cleanup(&dir);
    }
}
