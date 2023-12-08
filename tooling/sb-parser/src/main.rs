use nom::{
    branch::alt,
    bytes::complete::{tag, take_until},
    character::complete::{alpha1, alphanumeric1, anychar, space1},
    combinator::{peek, recognize},
    multi::{many0, many_till},
    sequence::{pair, tuple},
    IResult,
};
use std::path::Path;
use std::{fs, io::Write};

#[derive(Debug, Clone)]
struct GoType {
    pub name: String,
    pub def: GoTypeDef,
}

#[derive(Debug, Clone)]
enum GoTypeDef {
    Basic(String),
    Struct(String),
}

impl GoType {
    fn inner_content(&self) -> String {
        match &self.def {
            GoTypeDef::Basic(def) => def.clone(),
            GoTypeDef::Struct(def) => def.clone(),
        }
    }
}

fn variable(input: &str) -> IResult<&str, &str> {
    recognize(pair(
        alt((alpha1, tag("_"))),
        many0(alt((alphanumeric1, tag("_")))),
    ))(input)
}

fn parse_type(input: &str) -> IResult<&str, GoType> {
    let (input, _) = many_till(anychar, tag("type"))(input)?;
    let (input, (_, name, _)) = tuple((space1, variable, space1))(input)?;

    match peek::<_, _, nom::error::Error<_>, _>(tag("struct"))(input) {
        Ok((input, _)) => {
            let (input, _) = tuple((tag("struct"), space1, tag("{")))(input)?;
            let (input, def) = take_until("}")(input)?;
            let def = def.trim_end().trim_start_matches("\n").to_string();
            Ok((
                input,
                GoType {
                    name: name.to_string(),
                    def: GoTypeDef::Struct(def),
                },
            ))
        }
        Err(_) => {
            let (input, def) = take_until("\n")(input)?;
            let def = def.trim().to_string();
            Ok((
                input,
                GoType {
                    name: name.to_string(),
                    def: GoTypeDef::Basic(def.into()),
                },
            ))
        }
    }
}

fn main() {
    let root_dir = format!("{}/outputs", env!("CARGO_MANIFEST_DIR"));
    let root_dir = Path::new(&root_dir);

    let mut go_type_list: Vec<GoType> = vec![];

    let src_path = root_dir.join("sources");
    for entry in fs::read_dir(&src_path).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();

        if path.is_file() && path.extension().unwrap_or_default() == "go" {
            // println!("reading {}", path.file_name().unwrap().to_str().unwrap());
            let path = Path::new(&src_path).join(path);
            let go_code = fs::read_to_string(path).expect("Failed to read file");
            let mut go_code = go_code.as_str();

            'outer: loop {
                while let Ok((remaining, go_type)) = parse_type(go_code) {
                    go_code = remaining;
                    go_type_list.push(go_type);
                }
                if go_code.contains("type") {
                    let (remaining, _) =
                        take_until::<&str, &str, nom::error::Error<&str>>("type")(go_code).unwrap();
                    let (remaining, _) =
                        tag::<&str, &str, nom::error::Error<&str>>("type")(remaining).unwrap();
                    go_code = remaining;
                    continue;
                } else {
                    break 'outer;
                }
            }
        }
    }

    let outbound = go_type_list.iter().find(|x| x.name == "_Outbound").unwrap();

    let mut all_content = String::new();

    let mut set = std::collections::HashSet::<String>::new();

    for t in &go_type_list {
        if outbound.inner_content().contains(&t.name) {
            set.insert(t.name.clone());
            all_content.push_str(&t.inner_content());
            all_content.push_str("\n\n");
        }
    }

    loop {
        let mut change = false;
        for t in &go_type_list {
            if set.contains(&t.name) {
                continue;
            }

            println!("checking {}", t.name);

            let check1 = format!("{}", &t.name);
            let check2: String = format!("{} ", &t.name);

            if all_content.contains(&check1) || all_content.contains(&check2) {
                println!("-- [found] {}", t.name);

                set.insert(t.name.clone());
                all_content.push_str(&t.inner_content());
                all_content.push_str("\n\n");
                change = true;
            }
        }

        if !change {
            break;
        }
    }

    for s in &set {
        println!("{}", s);
    }

    let mut ret = vec![];

    for go_type in &go_type_list {
        if !set.contains(&go_type.name) {
            continue;
        }
        let temp = match &go_type.def {
            GoTypeDef::Basic(def) => format!("type {} {}", go_type.name, def),
            GoTypeDef::Struct(def) => format!("type {} struct {{\n{}\n}}", go_type.name, def),
        };
        ret.push(temp);
    }

    let ret = ret.join("\n\n");

    let mut file = std::fs::File::create(Path::new(&root_dir).join("outbounds.go")).unwrap();
    file.write_all(ret.as_bytes()).unwrap();
}
