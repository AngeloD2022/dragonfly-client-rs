use std::{
    io::{Cursor, Read},
    sync::RwLock,
};
use flate2::read::GzDecoder;
use reqwest::{blocking::Client, Url};
use tracing::info;
use yara::{Compiler, Rules};
use zip::ZipArchive;
use crate::{
    APP_CONFIG, 
    common::{TarballType, ZipType}, scanner::DistributionScanResults, api_models::SubmitJobResultsError,
};

use crate::{
    api_models::{
        AuthBody, 
        AuthResponse, 
        GetJobResponse, 
        GetRulesResponse, 
        Job, 
        SubmitJobResultsBody,
    },
    error::DragonflyError,
};

/// Application state
pub struct State {

    /// The current ruleset this client is using
    pub rules: yara::Rules,

    /// The GitHub commit hash of the ruleset this client is using
    pub hash: String,

    /// Access token this client is using for authentication
    pub access_token: String,
}

pub struct DragonflyClient {
    pub client: Client,
    pub state: RwLock<State>,
}

impl DragonflyClient {
    pub fn new() -> Result<Self, DragonflyError> {
        let client = Client::builder().gzip(true).build()?;

        let access_token = Self::fetch_access_token(&client)?;
        let (hash, rules) = Self::get_rules(&client, &access_token)?;
        let state = State { rules, hash, access_token }.into();

        Ok(Self {
            client,
            state,
        })
    }
    
    /// Fetch a new access token and set it in state
    pub fn reauthorize(&self) -> Result<(), reqwest::Error> {
        let access_token = Self::fetch_access_token(self.get_http_client())?;
        self.set_access_token(access_token);

        Ok(())
    }
    
    /// Fetch the latest ruleset and set it in state
    pub fn sync_rules(&self) -> Result<(), DragonflyError> {
        let access_token = &self.state.read().unwrap().access_token;
        let (hash, rules) = Self::get_rules(
            self.get_http_client(),
            access_token
        )?;

        let mut state = self.state.write().unwrap();
        state.hash = hash;
        state.rules = rules;

        Ok(())
    }
    
    /// Fetch a job. None if the server has nothing for us to do.
    pub fn get_job(&self) -> reqwest::Result<Option<Job>> {
        let access_token = &self.state.read().unwrap().access_token;
        let res: GetJobResponse = self
            .client
            .post(format!("{}/job", APP_CONFIG.base_url))
            .header("Authorization", format!("Bearer {access_token}"))
            .send()?
            .error_for_status()?
            .json()?;

        let job = match res {
            GetJobResponse::Job(job) => Some(job),
            GetJobResponse::Error { .. } => None,
        };

        Ok(job)
    }

    pub fn send_error(&self, job: &Job, reason: &str) -> Result<(), reqwest::Error> {
        let access_token = &self.state.read().unwrap().access_token; 

        let body = SubmitJobResultsError {
            name: &job.name,
            version: &job.version,
            reason,
        };

        self.client
            .put(format!("{}/package", APP_CONFIG.base_url))
            .header("Authorization", format!("Bearer {access_token}"))
            .json(&body)
            .send()?
            .error_for_status()?;

        Ok(())
    }
    
    /// Submit the results of a scan to the server, given the job and the scan results of each
    /// distribution
    pub fn submit_job_results(&self, job: &Job, distribution_scan_results: &[DistributionScanResults]) -> reqwest::Result<()> {
        let access_token = &self.state.read().unwrap().access_token;

        let highest_score_distribution = distribution_scan_results 
            .iter()
            .max_by_key(|distrib| distrib.get_total_score());
        
        let score = highest_score_distribution.map(DistributionScanResults::get_total_score).unwrap_or_default();
        let inspector_url = highest_score_distribution.and_then(DistributionScanResults::inspector_url);
        let rules_matched = highest_score_distribution.map(DistributionScanResults::get_matched_rule_identifiers).unwrap_or_default();

        let body = SubmitJobResultsBody {
            name: &job.name,
            version: &job.version,
            score,
            inspector_url: inspector_url.as_deref(),
            rules_matched: &rules_matched,
        };

        info!("{body:#?}");

        self.client
            .put(format!("{}/package", APP_CONFIG.base_url))
            .header("Authorization", format!("Bearer {access_token}"))
            .json(&body)
            .send()?
            .error_for_status()?;

        Ok(())
    }

    // Return a reference to the underlying HTTP Client
    pub fn get_http_client(&self) -> &Client {
        &self.client
    }

    pub fn set_access_token(&self, access_token: String) {
        let mut state = self.state.write().unwrap();
        state.access_token = access_token;
    }

    fn fetch_access_token(http_client: &Client) -> Result<String, reqwest::Error> {
        let url = format!("https://{}/oauth/token", APP_CONFIG.auth0_domain);
        let json_body = AuthBody {
            client_id: &APP_CONFIG.client_id,
            client_secret: &APP_CONFIG.client_secret,
            audience: &APP_CONFIG.audience,
            grant_type: &APP_CONFIG.grant_type,
            username: &APP_CONFIG.username,
            password: &APP_CONFIG.password,
        };

        let res: AuthResponse = http_client
            .post(url)
            .json(&json_body)
            .send()?
            .error_for_status()?
            .json()?;
        
        Ok(res.access_token)
    }

    fn get_rules(http_client: &Client, access_token: &str) -> Result<(String, Rules), DragonflyError> {
        let res: GetRulesResponse = http_client 
            .get(format!("{}/rules", APP_CONFIG.base_url))
            .header("Authorization", format!("Bearer {access_token}"))
            .send()?
            .error_for_status()?
            .json()?;

        let rules_str = res
            .rules
            .values()
            .cloned()
            .collect::<Vec<String>>()
            .join("\n");

        let compiled_rules = Compiler::new()?
            .add_rules_str(&rules_str)?
            .compile_rules()?;

        Ok((res.hash, compiled_rules))
    }
}

pub fn fetch_tarball(
    http_client: &Client,
    download_url: &Url,
) -> Result<TarballType, DragonflyError> {
    let response = http_client.get(download_url.to_owned()).send()?;

    let mut decompressed = GzDecoder::new(response);
    let mut cursor: Cursor<Vec<u8>> = Cursor::new(Vec::new());
    let read = decompressed.read_to_end(cursor.get_mut())?;

    if read > APP_CONFIG.max_scan_size {
        Err(DragonflyError::DownloadTooLarge(download_url.to_string()))
    } else {
        Ok(tar::Archive::new(cursor))
    }
}

pub fn fetch_zipfile(
    http_client: &Client, 
    download_url: &Url,
) -> Result<ZipType, DragonflyError> {
    let mut response = http_client.get(download_url.to_string()).send()?;

    let mut cursor = Cursor::new(Vec::new());
    let read = response.read_to_end(cursor.get_mut())?;

    if read > APP_CONFIG.max_scan_size {
        Err(DragonflyError::DownloadTooLarge(download_url.to_string()))
    } else {
        let zip = ZipArchive::new(cursor)?;
        Ok(zip)
    }
}

