#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Currency {
    pub prefix: String,
    pub suffix: String,
    pub decimal_separator: Option<char>,
    pub group_separator: Option<char>,
}

struct Sample {
    prefix: String,
    suffix: String,
    decimal_separator: Option<char>,
    group_separator: Option<Option<char>>,
}

impl Currency {
    pub fn italian() -> Currency {
        Currency {
            prefix: String::new(),
            suffix: "€".to_string(),
            decimal_separator: Some(','),
            group_separator: None,
        }
    }

    pub fn format(&self, cents: u32) -> String {
        let mut text = self.prefix.clone();
        text.push_str(&self.group(cents / 100));
        if let Some(separator) = self.decimal_separator {
            text.push(separator);
            text.push_str(&format!("{:02}", cents % 100));
        }
        text.push_str(&self.suffix);
        text
    }

    pub fn learn<'a>(samples: impl IntoIterator<Item = (u32, &'a str)>) -> Option<Currency> {
        let samples: Vec<Sample> = samples
            .into_iter()
            .filter_map(|(cents, text)| sample(cents, text))
            .collect();
        if samples.is_empty() {
            return None;
        }

        Some(Currency {
            prefix: most_common(samples.iter().map(|sample| sample.prefix.clone()))?,
            suffix: most_common(samples.iter().map(|sample| sample.suffix.clone()))?,
            decimal_separator: most_common(samples.iter().map(|sample| sample.decimal_separator))?,
            group_separator: most_common(
                samples.iter().filter_map(|sample| sample.group_separator),
            )
            .flatten(),
        })
    }

    fn group(&self, units: u32) -> String {
        let digits = units.to_string();
        let Some(separator) = self.group_separator else {
            return digits;
        };

        let mut grouped = String::new();
        for (index, digit) in digits.chars().enumerate() {
            if index > 0 && (digits.len() - index).is_multiple_of(3) {
                grouped.push(separator);
            }
            grouped.push(digit);
        }
        grouped
    }
}

fn sample(cents: u32, text: &str) -> Option<Sample> {
    if cents == 0 {
        return None;
    }
    let first = text.find(|c: char| c.is_ascii_digit())?;
    let last = text.rfind(|c: char| c.is_ascii_digit())?;
    let number: Vec<char> = text[first..=last].chars().collect();
    let value: u64 = number
        .iter()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;

    let (integer, decimal_separator) = if value == u64::from(cents) {
        let at = number.len().checked_sub(3)?;
        if number[at].is_ascii_digit() {
            return None;
        }
        (&number[..at], Some(number[at]))
    } else if value == u64::from(cents / 100) && cents.is_multiple_of(100) {
        (&number[..], None)
    } else {
        return None;
    };

    let mut separators = integer.iter().filter(|c| !c.is_ascii_digit());
    let group = separators.next().copied();
    if separators.any(|&separator| Some(separator) != group) {
        return None;
    }

    Some(Sample {
        prefix: text[..first].to_string(),
        suffix: text[last + 1..].to_string(),
        decimal_separator,
        group_separator: (cents / 100 >= 1000).then_some(group),
    })
}

fn most_common<T: PartialEq>(values: impl Iterator<Item = T>) -> Option<T> {
    let mut counts: Vec<(T, usize)> = Vec::new();
    for value in values {
        match counts.iter_mut().find(|(seen, _)| *seen == value) {
            Some((_, count)) => *count += 1,
            None => counts.push((value, 1)),
        }
    }

    let best = counts.iter().map(|(_, count)| *count).max()?;
    counts
        .into_iter()
        .find(|(_, count)| *count == best)
        .map(|(value, _)| value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEAM_PRICES: &[(&str, u32, &str)] = &[
        ("IT", 5999, "59,99€"),
        ("IT", 1399, "13,99€"),
        ("IT", 1119, "11,19€"),
        ("US", 5999, "$59.99"),
        ("US", 1499, "$14.99"),
        ("US", 1199, "$11.99"),
        ("JP", 902000, "¥9,020"),
        ("JP", 148000, "¥1,480"),
        ("JP", 136000, "¥1,360"),
        ("JP", 170000, "¥1,700"),
        ("JP", 849900, "¥8,499"),
        ("KR", 6480000, "₩ 64,800"),
        ("KR", 1600000, "₩ 16,000"),
        ("KR", 1320000, "₩ 13,200"),
        ("KR", 1650000, "₩ 16,500"),
        ("KR", 6600000, "₩ 66,000"),
        ("AR", 4799, "$47.99"),
        ("AR", 499, "$4.99"),
        ("AR", 639, "$6.39"),
        ("AR", 799, "$7.99"),
        ("AR", 3499, "$34.99"),
        ("TR", 3999, "$39.99"),
        ("TR", 599, "$5.99"),
        ("TR", 639, "$6.39"),
        ("TR", 799, "$7.99"),
        ("TR", 3499, "$34.99"),
        ("IN", 359900, "₹3,599.00"),
        ("IN", 47900, "₹479.00"),
        ("IN", 55200, "₹552.00"),
        ("IN", 69000, "₹690.00"),
        ("IN", 299900, "₹2,999.00"),
        ("ID", 59900000, "Rp 599 000"),
        ("ID", 11599900, "Rp 115 999"),
        ("ID", 10479900, "Rp 104 799"),
        ("ID", 13099900, "Rp 130 999"),
        ("ID", 69999900, "Rp 699 999"),
        ("CN", 29800, "¥298.00"),
        ("CN", 4800, "¥48.00"),
        ("CN", 4320, "¥43.20"),
        ("CN", 5400, "¥54.00"),
        ("CN", 26800, "¥268.00"),
        ("GB", 4999, "£49.99"),
        ("GB", 1099, "£10.99"),
        ("GB", 1023, "£10.23"),
        ("GB", 1279, "£12.79"),
        ("PL", 24900, "249,00 zł"),
        ("PL", 5399, "53,99 zł"),
        ("PL", 5199, "51,99 zł"),
        ("BR", 27450, "R$274,50"),
        ("BR", 2499, "R$24,99"),
        ("BR", 3599, "R$35,99"),
        ("BR", 4499, "R$44,99"),
        ("BR", 19999, "R$199,99"),
        ("VN", 99000000, "990.000₫"),
        ("VN", 16500000, "165.000₫"),
        ("VN", 16400000, "164.000₫"),
        ("VN", 20500000, "205.000₫"),
        ("RU", 359900, "3599 руб"),
        ("RU", 29900, "299 руб"),
        ("RU", 48800, "488 руб"),
        ("RU", 61000, "610 руб"),
        ("RU", 199900, "1999 руб"),
        ("UA", 179900, "1 799,00₴"),
        ("UA", 22900, "229,00₴"),
        ("UA", 26000, "260,00₴"),
        ("UA", 32500, "325,00₴"),
        ("UA", 89900, "899,00₴"),
        ("KZ", 1799900, "17 999,00₸"),
        ("KZ", 240000, "2 400,00₸"),
        ("KZ", 328000, "3 280,00₸"),
        ("KZ", 410000, "4 100,00₸"),
        ("KZ", 1400000, "14 000,00₸"),
        ("CH", 6990, "CHF 69.90"),
        ("CH", 1500, "CHF 15.00"),
        ("CH", 1343, "CHF 13.43"),
        ("CH", 1679, "CHF 16.79"),
        ("CH", 6999, "CHF 69.99"),
        ("NO", 59900, "599,00 kr"),
        ("NO", 10000, "100,00 kr"),
        ("NO", 13200, "132,00 kr"),
        ("NO", 16500, "165,00 kr"),
        ("CA", 7999, "C$ 79.99"),
        ("CA", 1699, "C$ 16.99"),
        ("CA", 1519, "C$ 15.19"),
        ("CA", 1899, "C$ 18.99"),
        ("AU", 8995, "A$ 89.95"),
        ("AU", 1699, "A$ 16.99"),
        ("AU", 1756, "A$ 17.56"),
        ("AU", 2195, "A$ 21.95"),
        ("NZ", 9999, "NZ$ 99.99"),
        ("NZ", 1799, "NZ$ 17.99"),
        ("NZ", 1759, "NZ$ 17.59"),
        ("NZ", 2199, "NZ$ 21.99"),
        ("MX", 97900, "Mex$ 979.00"),
        ("MX", 14999, "Mex$ 149.99"),
        ("MX", 14319, "Mex$ 143.19"),
        ("MX", 17899, "Mex$ 178.99"),
        ("MX", 129900, "Mex$ 1,299.00"),
        ("CL", 4549900, "CLP$45.499"),
        ("CL", 750000, "CLP$7.500"),
        ("CL", 664000, "CLP$6.640"),
        ("CL", 830000, "CLP$8.300"),
        ("CL", 3999900, "CLP$39.999"),
        ("CO", 20190000, "COL$ 201.900,00"),
        ("CO", 3200000, "COL$ 32.000,00"),
        ("CO", 3000000, "COL$ 30.000,00"),
        ("CO", 3750000, "COL$ 37.500,00"),
        ("CO", 19900000, "COL$ 199.000,00"),
        ("PE", 17500, "S/. 175.00"),
        ("PE", 3995, "S/. 39.95"),
        ("PE", 2680, "S/. 26.80"),
        ("PE", 3350, "S/. 33.50"),
        ("PE", 19900, "S/. 199.00"),
        ("UY", 202400, "$U2.024,00"),
        ("UY", 32900, "$U329,00"),
        ("UY", 36400, "$U364,00"),
        ("UY", 45500, "$U455,00"),
        ("UY", 159900, "$U1.599,00"),
        ("CR", 2425000, "₡24.250,00"),
        ("CR", 650000, "₡6.500,00"),
        ("CR", 520000, "₡5.200,00"),
        ("CR", 3490000, "₡34.900,00"),
        ("HK", 47900, "HK$ 479.00"),
        ("HK", 9900, "HK$ 99.00"),
        ("HK", 7600, "HK$ 76.00"),
        ("HK", 9500, "HK$ 95.00"),
        ("HK", 35800, "HK$ 358.00"),
        ("TW", 179000, "NT$ 1,790.00"),
        ("TW", 39800, "NT$ 398.00"),
        ("TW", 20600, "NT$ 206.00"),
        ("TW", 25800, "NT$ 258.00"),
        ("TW", 159900, "NT$ 1,599.00"),
        ("SG", 7990, "S$79.90"),
        ("SG", 1500, "S$15.00"),
        ("SG", 1160, "S$11.60"),
        ("SG", 1450, "S$14.50"),
        ("SG", 6900, "S$69.00"),
        ("TH", 179000, "฿1,790.00"),
        ("TH", 31500, "฿315.00"),
        ("TH", 25200, "฿252.00"),
        ("TH", 179900, "฿1,799.00"),
        ("MY", 19900, "RM199.00"),
        ("MY", 3100, "RM31.00"),
        ("MY", 3600, "RM36.00"),
        ("MY", 4500, "RM45.00"),
        ("MY", 21900, "RM219.00"),
        ("PH", 239900, "₱2,399.00"),
        ("PH", 41995, "₱419.95"),
        ("PH", 38800, "₱388.00"),
        ("PH", 48500, "₱485.00"),
        ("PH", 259900, "₱2,599.00"),
        ("SA", 24900, "249.00 SR"),
        ("SA", 4700, "47.00 SR"),
        ("SA", 2636, "26.36 SR"),
        ("SA", 3295, "32.95 SR"),
        ("SA", 22900, "229.00 SR"),
        ("AE", 24900, "249.00 AED"),
        ("AE", 4600, "46.00 AED"),
        ("AE", 3360, "33.60 AED"),
        ("AE", 4200, "42.00 AED"),
        ("AE", 22900, "229.00 AED"),
        ("IL", 24900, "₪249.00"),
        ("IL", 5595, "₪55.95"),
        ("IL", 4476, "₪44.76"),
        ("IL", 26900, "₪269.00"),
        ("ZA", 84900, "R 849.00"),
        ("ZA", 15900, "R 159.00"),
        ("ZA", 11600, "R 116.00"),
        ("ZA", 14500, "R 145.00"),
        ("ZA", 79900, "R 799.00"),
        ("QA", 24900, "249.00 QR"),
        ("QA", 3399, "33.99 QR"),
        ("QA", 2719, "27.19 QR"),
        ("QA", 23900, "239.00 QR"),
        ("KW", 1990, "19.90 KD"),
        ("KW", 280, "2.80 KD"),
        ("KW", 224, "2.24 KD"),
        ("KW", 1999, "19.99 KD"),
    ];

    fn country(code: &str) -> impl Iterator<Item = (u32, &'static str)> + '_ {
        STEAM_PRICES
            .iter()
            .filter(move |(country, _, _)| *country == code)
            .map(|(_, cents, text)| (*cents, *text))
    }

    fn italian_samples() -> [(u32, &'static str); 3] {
        [(5999, "59,99€"), (1399, "13,99€"), (1119, "11,19€")]
    }

    #[test]
    fn the_learned_currency_writes_every_steam_price_back() {
        let mut countries: Vec<&str> = STEAM_PRICES
            .iter()
            .map(|(country, _, _)| *country)
            .collect();
        countries.dedup();

        let mut checked = 0;
        for code in countries {
            let currency = Currency::learn(country(code)).unwrap();
            for (cents, text) in country(code) {
                assert_eq!(currency.format(cents), text, "{code}");
                checked += 1;
            }
        }
        assert_eq!(checked, 178);
    }

    #[test]
    fn the_italian_samples_learn_the_italian_currency() {
        assert_eq!(Currency::learn(country("IT")), Some(Currency::italian()));
    }

    #[test]
    fn the_most_common_value_wins() {
        let samples = italian_samples().into_iter().chain([(999, "9,99 €")]);

        assert_eq!(Currency::learn(samples), Some(Currency::italian()));
    }

    #[test]
    fn a_sample_that_does_not_add_up_is_dropped() {
        let samples = italian_samples().into_iter().chain([(5999, "5,99$")]);

        assert_eq!(Currency::learn(samples), Some(Currency::italian()));
        assert_eq!(Currency::learn([(5999, "5,99€"), (1399, "1.399€")]), None);
    }

    #[test]
    fn zero_cents_and_text_without_digits_are_dropped() {
        assert_eq!(Currency::learn([(0, "0,00€"), (999, "Free")]), None);
        assert_eq!(Currency::learn([]), None);
    }

    #[test]
    fn without_thousands_the_group_separator_is_none() {
        let currency = Currency::learn([(4999, "£49.99")]).unwrap();

        assert_eq!(currency.group_separator, None);
        assert_eq!(currency.format(123_456), "£1234.56");
    }

    #[test]
    fn format_groups_only_from_a_thousand_and_keeps_the_zero_cents() {
        let currency = Currency {
            prefix: String::new(),
            suffix: "₸".to_string(),
            decimal_separator: Some(','),
            group_separator: Some(' '),
        };

        assert_eq!(currency.format(99_900), "999,00₸");
        assert_eq!(currency.format(100_000), "1 000,00₸");
        assert_eq!(currency.format(123_456_789), "1 234 567,89₸");
        assert_eq!(Currency::italian().format(1500), "15,00€");
        assert_eq!(Currency::italian().format(5), "0,05€");
    }
}
