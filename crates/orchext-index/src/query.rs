use chrono::NaiveDate;

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub query: String,
    pub types: Vec<String>,
    pub tags: Vec<String>,
    pub allowed_visibility: Vec<String>,
    pub updated_since: Option<NaiveDate>,
    pub limit: u32,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            query: String::new(),
            types: Vec::new(),
            tags: Vec::new(),
            allowed_visibility: Vec::new(),
            updated_since: None,
            limit: 20,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub id: String,
    pub type_: String,
    pub title: String,
    pub snippet: String,
    pub score: f64,
    pub visibility: String,
    pub tags: Vec<String>,
    pub updated: Option<NaiveDate>,
}

#[derive(Debug, Clone)]
pub struct ListFilter {
    pub types: Vec<String>,
    pub tags: Vec<String>,
    pub allowed_visibility: Vec<String>,
    pub updated_since: Option<NaiveDate>,
    pub limit: u32,
}

impl Default for ListFilter {
    fn default() -> Self {
        Self {
            types: Vec::new(),
            tags: Vec::new(),
            allowed_visibility: Vec::new(),
            updated_since: None,
            limit: 100,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ListItem {
    pub id: String,
    pub type_: String,
    pub title: String,
    pub visibility: String,
    pub tags: Vec<String>,
    pub updated: Option<NaiveDate>,
}
