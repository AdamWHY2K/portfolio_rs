use chrono::prelude::*;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use yahoo_finance_api as yahoo;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PortfolioPosition {
    name: Option<String>,
    ticker: Option<String>,
    asset_class: String,
    amount: f64,

    // Interest rate fields for cash assets
    #[serde(skip_serializing_if = "Option::is_none", default)]
    interest_rate: Option<f64>, // Annual interest rate as percentage (e.g., 5.0 for 5%)
    
    #[serde(skip_serializing_if = "Option::is_none", default)]
    payment_frequency_days: Option<u32>, // How often interest is paid (in days)
    
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[serde(with = "optional_datetime_format")]
    last_interest_payment: Option<DateTime<Utc>>, // When interest was last paid
    
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[serde(with = "optional_datetime_format")]
    next_interest_payment: Option<DateTime<Utc>>, // When next interest payment is due

    #[serde(skip_deserializing)]
    last_spot: f64,
}

mod optional_datetime_format {
    use chrono::{DateTime, Utc, NaiveDate};
    use serde::{self, Deserialize, Deserializer, Serializer};

    const FORMAT: &str = "%Y-%m-%d";

    pub fn serialize<S>(
        date: &Option<DateTime<Utc>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match date {
            Some(d) => serializer.serialize_str(&d.format(FORMAT).to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: Option<String> = Option::deserialize(deserializer)?;
        match s {
            Some(s) => {
                if s.is_empty() {
                    Ok(None)
                } else {
                    NaiveDate::parse_from_str(&s, FORMAT)
                        .map(|d| Some(d.and_hms_opt(0, 0, 0).unwrap().and_utc()))
                        .map_err(serde::de::Error::custom)
                }
            }
            None => Ok(None),
        }
    }
}

impl PortfolioPosition {
    fn update_price(&mut self, last_spot: f64) {
        self.last_spot = last_spot;
    }

    pub fn get_name(&self) -> &str {
        if let Some(name) = &self.name {
            name
        } else if let Some(ticker) = &self.ticker {
            ticker
        } else {
            "Unknown"
        }
    }

    pub fn get_asset_class(&self) -> &str {
        &self.asset_class
    }

    pub fn get_balance(&self) -> f64 {
        if let Some(_ticker) = &self.ticker {
            self.last_spot * self.amount
        } else {
            self.amount
        }
    }

    pub fn get_amount(&self) -> f64 {
        self.amount
    }

    pub fn get_ticker(&self) -> Option<&str> {
        self.ticker.as_deref()
    }

    pub fn get_name_option(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn set_amount(&mut self, amount: f64) {
        self.amount = amount;
    }

    // Interest-related methods for cash assets
    pub fn get_interest_rate(&self) -> Option<f64> {
        self.interest_rate
    }

    pub fn get_payment_frequency_days(&self) -> Option<u32> {
        self.payment_frequency_days
    }

    pub fn get_last_interest_payment(&self) -> Option<DateTime<Utc>> {
        self.last_interest_payment
    }

    pub fn get_next_interest_payment(&self) -> Option<DateTime<Utc>> {
        self.next_interest_payment
    }

    pub fn set_interest_rate(&mut self, rate: Option<f64>) {
        self.interest_rate = rate;
    }

    pub fn set_payment_frequency_days(&mut self, days: Option<u32>) {
        self.payment_frequency_days = days;
    }

    pub fn set_last_interest_payment(&mut self, date: Option<DateTime<Utc>>) {
        self.last_interest_payment = date;
    }

    pub fn set_next_interest_payment(&mut self, date: Option<DateTime<Utc>>) {
        self.next_interest_payment = date;
    }

    /// Check if this is a cash asset that earns interest
    pub fn is_cash_with_interest(&self) -> bool {
        self.asset_class.to_lowercase() == "cash" 
            && self.interest_rate.is_some() 
            && self.payment_frequency_days.is_some()
            && self.next_interest_payment.is_some()
    }

    /// Calculate the daily interest amount
    pub fn daily_interest_amount(&self) -> f64 {
        if let Some(rate) = self.interest_rate {
            self.amount * (rate / 100.0) / 365.0
        } else {
            0.0
        }
    }

    /// Calculate interest earned between two dates
    pub fn calculate_interest(&self, from_date: DateTime<Utc>, to_date: DateTime<Utc>) -> f64 {
        if let Some(rate) = self.interest_rate {
            let days = (to_date - from_date).num_days() as f64;
            self.amount * (rate / 100.0) * (days / 365.0)
        } else {
            0.0
        }
    }

    /// Check if interest payment is due and apply it
    pub fn apply_interest_if_due(&mut self, current_date: DateTime<Utc>) -> Option<f64> {
        if !self.is_cash_with_interest() {
            return None;
        }

        if let Some(next_payment) = self.next_interest_payment {
            if current_date >= next_payment {
                let last_payment = self.last_interest_payment.unwrap_or(next_payment);
                let interest = self.calculate_interest(last_payment, current_date);
                
                // Add interest to the principal amount
                self.amount += interest;
                
                // Update payment dates
                self.last_interest_payment = Some(current_date);
                self.next_interest_payment = Some(self.calculate_next_payment_date(current_date));
                
                return Some(interest);
            }
        }

        None
    }

    /// Calculate the next interest payment date using the configured frequency
    fn calculate_next_payment_date(&self, current_date: DateTime<Utc>) -> DateTime<Utc> {
        let frequency_days = self.payment_frequency_days.unwrap_or(30); // Fallback to 30 days
        current_date + chrono::Duration::days(frequency_days as i64)
    }
}

pub fn from_string(data: &str) -> Vec<PortfolioPosition> {
    serde_json::from_str::<Vec<PortfolioPosition>>(data).expect("JSON was not well-formatted")
}

// Get the latest price for a ticker
async fn get_quote_price(ticker: &str) -> Result<yahoo::YResponse, yahoo::YahooError> {
    yahoo::YahooConnector::new()?
        .get_latest_quotes(ticker, "1d")
        .await
}

// get the price at a given date
pub async fn get_historic_price(
    ticker: &str,
    date: DateTime<Utc>,
) -> Result<yahoo::YResponse, yahoo::YahooError> {
    let start = OffsetDateTime::from_unix_timestamp(date.timestamp()).unwrap();

    // get a range of 3 days in case the market is closed on the given date
    let end = start + time::Duration::days(3);

    yahoo::YahooConnector::new()?
        .get_quote_history(ticker, start, end)
        .await
}

// Try to get the short name for a ticker from Yahoo Finance
async fn get_quote_name(ticker: &str) -> Result<String, yahoo::YahooError> {
    let connector = yahoo::YahooConnector::new();
    let resp = connector?.search_ticker(ticker).await?;

    if let Some(item) = resp.quotes.first() {
        Ok(item.short_name.clone())
    } else {
        Err(yahoo::YahooError::NoResult)
    }
}

// Get the latest price for a ticker and update the positionthen
// then return the updated position as a new object
pub async fn handle_position(
    position: &mut PortfolioPosition,
) -> Result<PortfolioPosition, yahoo::YahooError> {
    if let Some(ticker) = &position.ticker {
        let quote = get_quote_price(ticker).await?;
        if let Ok(last_spot) = quote.last_quote() {
            position.update_price(last_spot.close)
        } else {
            // if the market is closed, try to get the last available price
            if let Ok(last_spot) = quote.quotes() {
                if let Some(last_spot) = last_spot.last() {
                    position.update_price(last_spot.close);
                }
            }
        }

        // if no name was provided in the JSON, try to get it from Yahoo Finance
        if position.name.is_none() {
            if let Some(ticker) = &position.ticker {
                let name = get_quote_name(ticker).await?;
                position.name = Some(name);
            }
        }
    }

    Ok(PortfolioPosition {
        name: position.name.clone(),
        ticker: position.ticker.to_owned(),
        asset_class: position.asset_class.to_string(),
        amount: position.amount,
        interest_rate: position.interest_rate,
        payment_frequency_days: position.payment_frequency_days,
        last_interest_payment: position.last_interest_payment,
        next_interest_payment: position.next_interest_payment,
        last_spot: position.last_spot,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn test_get_quote_name() {
        let name = get_quote_name("AAPL").await.unwrap();
        assert_eq!(name, "Apple Inc.");

        let name = get_quote_name("BTC-EUR").await.unwrap();
        assert_eq!(name, "Bitcoin EUR");
    }

    #[tokio::test]
    async fn test_get_quote_price() {
        let quote = get_quote_price("AAPL").await.unwrap();
        assert!(quote.last_quote().unwrap().close > 0.0);
    }

    #[tokio::test]
    async fn test_get_historic_price() {
        let date = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        let quote = get_historic_price("AAPL", date).await.unwrap();
        assert_eq!(
            quote.quotes().unwrap().last().unwrap().close,
            74.35749816894531
        );
    }

    #[tokio::test]
    async fn test_handle_position() {
        let mut position = PortfolioPosition {
            name: None,
            ticker: Some("AAPL".to_string()),
            asset_class: "Stock".to_string(),
            amount: 1.0,
            interest_rate: None,
            payment_frequency_days: None,
            last_interest_payment: None,
            next_interest_payment: None,
            last_spot: 0.0,
        };

        let updated_position = handle_position(&mut position)
            .await
            .expect("Error handling position");
        assert_eq!(updated_position.get_name(), "Apple Inc.");
        assert_eq!(
            updated_position.get_balance(),
            updated_position.get_amount() * updated_position.last_spot
        );
    }

    #[tokio::test]
    async fn test_from_file() {
        let positions_str = fs::read_to_string("test_cash.json").unwrap();
        let positions = from_string(&positions_str);
        assert_eq!(positions.len(), 2);
        
        // Check that the first cash position has interest rate information
        let cash_position = &positions[0];
        assert!(cash_position.interest_rate.is_some());
        assert_eq!(cash_position.interest_rate.unwrap(), 5.0);
        assert!(cash_position.last_interest_payment.is_some());
        assert!(cash_position.next_interest_payment.is_some());
        
        // Check that the second cash position does not have interest rate
        let regular_cash = &positions[1];
        assert!(regular_cash.interest_rate.is_none());
    }
    
    #[tokio::test]
    async fn test_from_example_file() {
        let positions_str = fs::read_to_string("example_data.json").unwrap();
        let positions = from_string(&positions_str);
        assert_eq!(positions.len(), 6);
        
        // Check that the cash position has interest rate information
        let cash_position = positions.iter().find(|p| p.asset_class == "Cash").unwrap();
        assert!(cash_position.interest_rate.is_some());
        assert_eq!(cash_position.interest_rate.unwrap(), 4.5);
        assert!(cash_position.last_interest_payment.is_some());
        assert!(cash_position.next_interest_payment.is_some());
    }

    #[tokio::test]
    async fn test_interest_calculation() {
        let mut position = PortfolioPosition {
            name: Some("Test Savings".to_string()),
            ticker: None,
            asset_class: "Cash".to_string(),
            amount: 1000.0,
            interest_rate: Some(5.0), // 5% APY
            payment_frequency_days: Some(30), // Monthly payments
            last_interest_payment: None,
            next_interest_payment: None,
            last_spot: 0.0,
        };

        // Test daily interest calculation
        let daily_interest = position.daily_interest_amount();
        let expected_daily = 1000.0 * 0.05 / 365.0;
        assert!((daily_interest - expected_daily).abs() < 0.01);

        // Test interest calculation between dates
        let start_date = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        let end_date = Utc.with_ymd_and_hms(2025, 1, 31, 0, 0, 0).unwrap(); // 30 days
        let interest = position.calculate_interest(start_date, end_date);
        let expected_interest = 1000.0 * 0.05 * (30.0 / 365.0);
        assert!((interest - expected_interest).abs() < 0.01);

        // Test interest payment application with explicit payment dates
        let current_date = Utc::now();
        let last_payment_date = current_date - chrono::Duration::days(30);
        let next_payment_date = current_date - chrono::Duration::days(1); // Payment is due
        
        position.last_interest_payment = Some(last_payment_date);
        position.next_interest_payment = Some(next_payment_date);
        
        let paid_interest = position.apply_interest_if_due(current_date);
        assert!(paid_interest.is_some());
        assert!(position.amount > 1000.0); // Amount should have increased
        
        // Verify the payment was recorded
        assert!(position.last_interest_payment.is_some());
        assert!(position.next_interest_payment.is_some());
        
        // Verify next payment date is calculated using frequency
        let next_payment = position.next_interest_payment.unwrap();
        let expected_next = current_date + chrono::Duration::days(30);
        let diff = (next_payment - expected_next).num_seconds().abs();
        assert!(diff < 60); // Should be within a minute
    }
}
