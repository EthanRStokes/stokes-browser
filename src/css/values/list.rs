// CSS list-style-type property

/// CSS list-style-type property
#[derive(Debug, Clone, PartialEq)]
pub enum ListStyleType {
    None,
    Disc,
    Circle,
    Square,
    Decimal,
    DecimalLeadingZero,
    LowerRoman,
    UpperRoman,
    LowerAlpha,
    UpperAlpha,
    LowerGreek,
    LowerLatin,
    UpperLatin,
}

impl ListStyleType {
    /// Parse list-style-type value from string
    pub fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "none" => ListStyleType::None,
            "disc" => ListStyleType::Disc,
            "circle" => ListStyleType::Circle,
            "square" => ListStyleType::Square,
            "decimal" => ListStyleType::Decimal,
            "decimal-leading-zero" => ListStyleType::DecimalLeadingZero,
            "lower-roman" => ListStyleType::LowerRoman,
            "upper-roman" => ListStyleType::UpperRoman,
            "lower-alpha" | "lower-latin" => ListStyleType::LowerAlpha,
            "upper-alpha" | "upper-latin" => ListStyleType::UpperAlpha,
            "lower-greek" => ListStyleType::LowerGreek,
            _ => ListStyleType::Disc, // Default to disc
        }
    }

    /// Get the marker/bullet for a given list item index (1-based)
    pub fn get_marker(&self, index: usize) -> String {
        match self {
            ListStyleType::None => String::new(),
            ListStyleType::Disc => "•".to_string(),
            ListStyleType::Circle => "◦".to_string(),
            ListStyleType::Square => "▪".to_string(),
            ListStyleType::Decimal => format!("{}.", index),
            ListStyleType::DecimalLeadingZero => format!("{:02}.", index),
            ListStyleType::LowerRoman => format!("{}.", Self::to_lower_roman(index)),
            ListStyleType::UpperRoman => format!("{}.", Self::to_upper_roman(index)),
            ListStyleType::LowerAlpha | ListStyleType::LowerLatin => {
                format!("{}.", Self::to_lower_alpha(index))
            }
            ListStyleType::UpperAlpha | ListStyleType::UpperLatin => {
                format!("{}.", Self::to_upper_alpha(index))
            }
            ListStyleType::LowerGreek => format!("{}.", Self::to_lower_greek(index)),
        }
    }

    /// Convert number to lowercase alphabetic representation (a, b, c, ...)
    fn to_lower_alpha(num: usize) -> String {
        if num == 0 {
            return String::new();
        }
        let mut result = String::new();
        let mut n = num;
        while n > 0 {
            n -= 1;
            result.insert(0, (b'a' + (n % 26) as u8) as char);
            n /= 26;
        }
        result
    }

    /// Convert number to uppercase alphabetic representation (A, B, C, ...)
    fn to_upper_alpha(num: usize) -> String {
        if num == 0 {
            return String::new();
        }
        let mut result = String::new();
        let mut n = num;
        while n > 0 {
            n -= 1;
            result.insert(0, (b'A' + (n % 26) as u8) as char);
            n /= 26;
        }
        result
    }

    /// Convert number to lowercase Roman numerals
    fn to_lower_roman(num: usize) -> String {
        Self::to_roman(num).to_lowercase()
    }

    /// Convert number to uppercase Roman numerals
    fn to_upper_roman(num: usize) -> String {
        Self::to_roman(num)
    }

    /// Convert number to Roman numerals
    fn to_roman(num: usize) -> String {
        if num == 0 {
            return String::new();
        }
        let values = [
            (1000, "M"),
            (900, "CM"),
            (500, "D"),
            (400, "CD"),
            (100, "C"),
            (90, "XC"),
            (50, "L"),
            (40, "XL"),
            (10, "X"),
            (9, "IX"),
            (5, "V"),
            (4, "IV"),
            (1, "I"),
        ];

        let mut result = String::new();
        let mut n = num;

        for (value, symbol) in values.iter() {
            while n >= *value {
                result.push_str(symbol);
                n -= value;
            }
        }

        result
    }

    /// Convert number to lowercase Greek letters
    fn to_lower_greek(num: usize) -> String {
        if num == 0 {
            return String::new();
        }
        // Greek alphabet: α, β, γ, δ, ε, ζ, η, θ, ι, κ, λ, μ, ν, ξ, ο, π, ρ, σ, τ, υ, φ, χ, ψ, ω
        let greek = ['α', 'β', 'γ', 'δ', 'ε', 'ζ', 'η', 'θ', 'ι', 'κ', 'λ', 'μ',
                     'ν', 'ξ', 'ο', 'π', 'ρ', 'σ', 'τ', 'υ', 'φ', 'χ', 'ψ', 'ω'];

        if num <= greek.len() {
            greek[num - 1].to_string()
        } else {
            // For numbers beyond the Greek alphabet, cycle through
            let idx = (num - 1) % greek.len();
            format!("{}{}", greek[idx], (num - 1) / greek.len() + 1)
        }
    }
}

impl Default for ListStyleType {
    fn default() -> Self {
        ListStyleType::Disc
    }
}

