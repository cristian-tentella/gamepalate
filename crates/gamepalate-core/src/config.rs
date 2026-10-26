#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    pub signals: SignalsConfig,
    pub relevance: RelevanceConfig,
    pub results: ResultsConfig,
    pub corrections: CorrectionsConfig,
    pub prices_max_age_hours: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            signals: SignalsConfig::default(),
            relevance: RelevanceConfig::default(),
            results: ResultsConfig::default(),
            corrections: CorrectionsConfig::default(),
            prices_max_age_hours: 24,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SignalsConfig {
    pub min_hours: f64,
    pub max_hours: f64,
    pub recent_hours_multiplier: f64,
    pub wishlist_hours: f64,
    pub followed_hours: f64,
    pub would_try_hours: f64,
}

impl Default for SignalsConfig {
    fn default() -> Self {
        SignalsConfig {
            min_hours: 2.0,
            max_hours: 200.0,
            recent_hours_multiplier: 3.0,
            wishlist_hours: 5.0,
            followed_hours: 3.0,
            would_try_hours: 5.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelevanceConfig {
    pub quality_weight: f64,
    pub min_reviews: u32,
    pub votes_weight: f64,
}

impl Default for RelevanceConfig {
    fn default() -> Self {
        RelevanceConfig {
            quality_weight: 0.5,
            min_reviews: 1000,
            votes_weight: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResultsConfig {
    pub count: usize,
    pub early_access: bool,
    pub min_positive_percent: u8,
    pub bet_every: usize,
}

impl Default for ResultsConfig {
    fn default() -> Self {
        ResultsConfig {
            count: 20,
            early_access: true,
            min_positive_percent: 0,
            bet_every: 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CorrectionsConfig {
    pub boost: f64,
    pub dampen: f64,
}

impl Default for CorrectionsConfig {
    fn default() -> Self {
        CorrectionsConfig {
            boost: 2.0,
            dampen: 0.5,
        }
    }
}
