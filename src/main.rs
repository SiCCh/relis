use std::io::BufRead;
use std::path::PathBuf;
use glob::glob;
use std::str::FromStr;
use std::collections::BTreeMap;
use polars::prelude::*;
use std::error::Error;
use regex::Regex;
//use core::fmt::Error;
//use csv::Error;
//use regex::Error;


// A CLI tool that extracts values from AMBER MD .lis files.
fn main() -> Result<(), Box<dyn Error>> {
    // Arguments parsing.
    let args = parse_args()?;
    let pattern = args.0;
    let path = PathBuf::from_str(&args.1)?;
    extract_all_values(&pattern, &path)?;
    Ok(())
}

fn extract_all_values(pattern: &String, path: &PathBuf) -> Result<(), Box<dyn Error>> {
    println!("Searching pattern \"{}\" in directory {}", pattern, path.display());
    let files = list_files(&path, &pattern)?;
    println!("Files found: {}", files.len());
    let mut df = DataFrame::new::<Series>(vec![])?;
    for file in files {
        println!("Reading file {}", file.display());
        let lines = read_lines_until_pattern(&file, "RESULTS", "A V E R A G E");
        let data = extract_values(&lines?)?;
        let mut temp_df = DataFrame::new::<Series>(vec![])?;
        // Iterate over the BTreeMap and create a new column for each key/values pair.
        for (key, values) in data.iter() {
            let s = Series::new(&key, values);
            temp_df.with_column(s)?;
        }
        df = df.vstack(&temp_df)?;
    }
    // If there is nothing, exit.
    if df.is_empty() {
        println!("No data found.");
        std::process::exit(0);
    }
    // Check if a column named "TIME(PS)" exists. 
    // If true, put it in first position and sort the values in ascending time order.
    let mut col = df.get_column_names();
    if let Some(pos) = &col.iter().position(|x| *x == "TIME(PS)") {
        col.remove(*pos);
        col.insert(0, "TIME(PS)");
        df = df.select(&col)?;
        df.sort_in_place(["TIME(PS)"], false)?;
    }
    let csv_path = path.join("LISFILES_SUMMARY.CSV");
    let mut csv_file = std::fs::File::create(&csv_path)?;
    CsvWriter::new(&mut csv_file).finish(&mut df)?;
    println!("Data saved in {}", csv_path.display());
    // Print the mean and standard deviation for each column in the terminal.
    let mut summary = df.mean();
    summary = summary.vstack(&df.std(0))?;
    for col in summary.get_column_names() {
        println!("          {}\n\nMean=     {}\nStd=      {}\n------------------------------",
        col,
        summary.column(col).unwrap().f64().unwrap().get(0).unwrap(),
        summary.column(col).unwrap().f64().unwrap().get(1).unwrap());
    }
    Ok(())
}

// Parses argument(s) from the command line, return the pattern used to select files to read and the path to the search directory.
fn parse_args() -> Result<(String, String), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        return Err("Not enough arguments provided. Usage: relis \"path/to/directory/pattern\" (glob style)".into());
    }
    if args[1] == "help" {
        println!("Usage: relis \"path/to/directory/pattern\" (glob style)");
        std::process::exit(0);
    }
    let path: PathBuf = PathBuf::from(args[1].clone());
    let pattern: String = path
        .file_name()
        .ok_or("Failed to extract file name from the provided path")?
        .to_str()
        .ok_or("File name is not valid UTF-8")?
        .to_string();
    let mut dir = std::env::current_dir()?
        .to_str()
        .ok_or("Current directory path is not valid UTF-8")?
        .to_string();
    if let Some(parent) = path.parent() {
        dir = parent
        .to_str()
        .ok_or("Parent directory path is not valid UTF-8")?
        .to_string();
    }
    Ok((pattern, dir))
}

// List all files containing a specific pattern in their names in the specified path.
// Returns a vector of strings containing the names of the files.
// 1st arg: Path to the directory in which the files are to be searched.
// 2nd arg: The pattern to be searched in the file names.
fn list_files(path: &PathBuf, pattern: &str) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut files = Vec::new();
    let pattern_str = format!("{}/{}", path.display(), pattern);
    for entry in glob(&pattern_str)? {
        match entry {
            Ok(path) => files.push(path),
            Err(e) => println!("{:?}", e),
        }
    }
    // Print the list of files found.
    for file in &files {
        println!("{}", file.display());
    }
    Ok(files)
}

// Open a text file and read lines from it, keep only lines between two patterns.
// Returns a vector of strings containing the lines in the file between the two patterns.
// 1st arg: Path to the file to be read.
// 2nd arg: The pattern that marks the beginning of the lines to be kept.
// 3rd arg: The pattern that marks the end of the lines to be kept.
fn read_lines_until_pattern(file_path: &PathBuf, pattern_start: &str, pattern_end: &str ) -> Result<Vec<String>, Box<dyn Error>> {
    let file = std::fs::File::open(file_path)?;
    let reader = std::io::BufReader::new(file);
    let mut lines = Vec::new();
    let mut start = false;
    for line in reader.lines() {
        let line = line?;
        if line.contains(pattern_start) {
            start = true;
        }
        if line.contains(pattern_end) {
            break;
        }
        if start {
            lines.push(line);
        }
    }
    Ok(lines)
}

// A function that extract the list of the differents values available for each frame in the .lis file using regex.
// The function returns a Result with a BTreeMap of <String, float> that contains the names of the name and values for each frame, or an error.
// 1st arg: A vector of strings containing the lines that contain the values.
fn extract_values(lines: &Vec<String>) -> Result<BTreeMap<String, Vec<f64>>, Box<dyn Error>> {
    // Create a BTreeMap that will contain the types of values and their values for each frame.
    let mut data = BTreeMap::new();
    // Create a regex to capture the categories and the values.
    let re = Regex::new(r"([1\-4\s]*[A-Za-z]+[\(A-Z)]*)\s+=\s+([-]?\d+[\.]?\d*)")?;
    // For each line, search and add the value to the corresponding key in the data map.
    for line in lines {
        for cap in re.captures_iter(line) {
            if line.contains("KE") || line.contains("err") {
                continue;
            }
            let t = cap[1].trim().to_string();
            // Convert the value to a float
            let v = cap[2].parse::<f64>()?;
            data.entry(t).or_insert(Vec::new()).push(v);
        }
    }
    Ok(data)
}