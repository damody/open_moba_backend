use regex::Regex;
use std::error::Error;
use std::fs;

/// JSON 預處理器，支援 C-style 註解
pub struct JsonPreprocessor;

impl JsonPreprocessor {
    /// 移除 JSON 字串中的 C-style 註解
    /// 支援 // 單行註解和 /* */ 多行註解
    pub fn remove_comments(json_str: &str) -> String {
        // 先處理多行註解，避免影響單行註解的處理
        let json_str = Self::remove_multiline_comments(json_str);
        
        // 再處理單行註解
        Self::remove_single_line_comments(&json_str)
    }
    
    /// 移除單行註解 //
    fn remove_single_line_comments(json_str: &str) -> String {
        let mut result = String::new();
        let mut in_string = false;
        let mut escape_next = false;
        let lines = json_str.lines();
        
        for line in lines {
            let mut processed_line = String::new();
            let chars: Vec<char> = line.chars().collect();
            let mut i = 0;
            
            while i < chars.len() {
                if escape_next {
                    processed_line.push(chars[i]);
                    escape_next = false;
                    i += 1;
                    continue;
                }
                
                if chars[i] == '\\' && in_string {
                    escape_next = true;
                    processed_line.push(chars[i]);
                    i += 1;
                    continue;
                }
                
                if chars[i] == '"' {
                    in_string = !in_string;
                    processed_line.push(chars[i]);
                    i += 1;
                    continue;
                }
                
                // 檢查是否為 // 註解開始
                if !in_string && i + 1 < chars.len() && chars[i] == '/' && chars[i + 1] == '/' {
                    // 忽略該行剩餘部分
                    break;
                }
                
                processed_line.push(chars[i]);
                i += 1;
            }
            
            // 只添加非空行
            let trimmed = processed_line.trim();
            if !trimmed.is_empty() {
                result.push_str(&processed_line);
                result.push('\n');
            }
        }
        
        result
    }
    
    /// 移除多行註解 /* */
    fn remove_multiline_comments(json_str: &str) -> String {
        let mut result = String::new();
        let chars: Vec<char> = json_str.chars().collect();
        let mut i = 0;
        let mut in_string = false;
        let mut escape_next = false;
        
        while i < chars.len() {
            if escape_next {
                result.push(chars[i]);
                escape_next = false;
                i += 1;
                continue;
            }
            
            if chars[i] == '\\' && in_string {
                escape_next = true;
                result.push(chars[i]);
                i += 1;
                continue;
            }
            
            if chars[i] == '"' {
                in_string = !in_string;
                result.push(chars[i]);
                i += 1;
                continue;
            }
            
            // 檢查是否為 /* 註解開始
            if !in_string && i + 1 < chars.len() && chars[i] == '/' && chars[i + 1] == '*' {
                // 找到對應的 */
                i += 2;
                while i + 1 < chars.len() {
                    if chars[i] == '*' && chars[i + 1] == '/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            
            result.push(chars[i]);
            i += 1;
        }
        
        result
    }
    
    /// 從檔案讀取並解析支援註解的 JSON
    pub fn read_json_with_comments<T>(file_path: &str) -> Result<T, Box<dyn Error>>
    where
        T: serde::de::DeserializeOwned,
    {
        let json_str = fs::read_to_string(file_path)?;
        let cleaned_json = Self::remove_comments(&json_str);
        let result = serde_json::from_str(&cleaned_json)?;
        Ok(result)
    }
    
    /// 從字串解析支援註解的 JSON
    pub fn parse_json_with_comments<T>(json_str: &str) -> Result<T, Box<dyn Error>>
    where
        T: serde::de::DeserializeOwned,
    {
        let cleaned_json = Self::remove_comments(json_str);
        let result = serde_json::from_str(&cleaned_json)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    
    #[test]
    fn test_single_line_comments() {
        let json_with_comments = r#"
        {
            // 這是一個註解
            "name": "test", // 行尾註解
            "value": 123
        }
        "#;
        
        let cleaned = JsonPreprocessor::remove_comments(json_with_comments);
        let result: Result<Value, _> = serde_json::from_str(&cleaned);
        assert!(result.is_ok());
        
        let value = result.unwrap();
        assert_eq!(value["name"], "test");
        assert_eq!(value["value"], 123);
    }
    
    #[test]
    fn test_multiline_comments() {
        let json_with_comments = r#"
        {
            /* 這是一個
               多行註解 */
            "name": "test",
            "value": /* 行內註解 */ 123
        }
        "#;
        
        let cleaned = JsonPreprocessor::remove_comments(json_with_comments);
        let result: Result<Value, _> = serde_json::from_str(&cleaned);
        assert!(result.is_ok());
        
        let value = result.unwrap();
        assert_eq!(value["name"], "test");
        assert_eq!(value["value"], 123);
    }
    
    #[test]
    fn test_comments_in_strings() {
        let json_with_comments = r#"
        {
            "url": "http://example.com", // 這個註解應該被移除
            "comment": "這裡的 // 不是註解",
            "multiline": "這裡的 /* 也不是註解 */"
        }
        "#;
        
        let cleaned = JsonPreprocessor::remove_comments(json_with_comments);
        let result: Result<Value, _> = serde_json::from_str(&cleaned);
        assert!(result.is_ok());
        
        let value = result.unwrap();
        assert_eq!(value["url"], "http://example.com");
        assert_eq!(value["comment"], "這裡的 // 不是註解");
        assert_eq!(value["multiline"], "這裡的 /* 也不是註解 */");
    }
}