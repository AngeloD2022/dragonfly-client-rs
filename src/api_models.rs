use serde::Serialize;
use serde::{self, Deserialize};
use std::collections::HashMap;
use std::fmt::Display;
use yara::{Compiler, Rules};

use crate::error::DragonflyError;

#[derive(Debug, Serialize)]
pub struct SubmitJobResultsSuccess {
    pub name: String,
    pub version: String,
    pub score: i64,
    pub inspector_url: Option<String>,

    /// Contains all rule identifiers matched for the entire release.
    pub rules_matched: Vec<String>,

    /// The commit hash of the ruleset used to produce these results.
    pub commit: String,
}

impl Display for SubmitJobResultsSuccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Name: {}\n", self.name)?;
        write!(f, "Version: {}\n", self.version)?;
        write!(f, "Score: {}\n", self.score)?;
        write!(f, "Inspector URL: {}\n", &self.inspector_url.as_deref().unwrap_or("None"))?;
        write!(f, "Rules matched: {}\n", self.rules_matched.join(", "))?;
        write!(f, "Commit hash: {}\n", self.commit)?;

        Ok(())
    } 
}

#[derive(Debug, Serialize)]
pub struct SubmitJobResultsError {
    pub name: String,
    pub version: String,
    pub reason: String,
}

impl Display for SubmitJobResultsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Name: {}\n", self.name)?;
        write!(f, "Version: {}\n", self.version)?;
        write!(f, "Reason: {}\n", self.reason)?;

        Ok(())
    } 
}

pub enum SubmitJobResultsBody {
    Success(SubmitJobResultsSuccess),
    Error(SubmitJobResultsError),
}

#[derive(Debug, Deserialize)]
pub struct Job {
    pub hash: String,
    pub name: String,
    pub version: String,
    pub distributions: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct GetRulesResponse {
    pub hash: String,
    pub rules: HashMap<String, String>,
}

impl GetRulesResponse {
    /// Compile the rules from the response
    pub fn compile(&self) -> Result<Rules, DragonflyError> {
        let rules_str = self
            .rules
            .values()
            .map(String::as_ref)
            .collect::<Vec<&str>>()
            .join("\n");

        let compiled_rules = Compiler::new()?
            .add_rules_str(&rules_str)?
            .compile_rules()?;

        Ok(compiled_rules)
    }
}

#[derive(Debug, Deserialize)]
pub struct AuthResponse {
    pub access_token: String,
    pub expires_in: u32,
    pub token_type: String,
}

#[derive(Debug, Serialize)]
pub struct AuthBody<'a> {
    pub client_id: &'a str,
    pub client_secret: &'a str,
    pub audience: &'a str,
    pub grant_type: &'a str,
    pub username: &'a str,
    pub password: &'a str,
}
