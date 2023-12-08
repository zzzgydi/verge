import os
import dotenv
from openai import OpenAI

dotenv.load_dotenv(verbose=True)


print(os.getenv("OPENAI_API_BASE"))
print(os.getenv("OPENAI_API_KEY"))

client = OpenAI(
    api_key=os.getenv("OPENAI_API_KEY"),
    base_url=os.getenv("OPENAI_API_BASE"),
)

system_prompt = "You are a Rust and Golang Expect, you are going to help me to translate the Go code to Rust code."
user_prompt = """Translate the following Go code to Rust code, Only translate the type define which name contains `Outbound` and its related type:

# Go Code

```go
{code}
```

# Rust code

Output the Rust code directly and NOTHING else!

```rust
"""


if __name__ == "__main__":
    dir_path = os.path.abspath(
        os.path.join(os.path.dirname(os.path.abspath(__file__)), "../outputs")
    )

    go_path = os.path.join(dir_path, "outbounds.go")
    outbound_code = ""
    with open(go_path, "r") as f:
        outbound_code = f.read()

    res = client.chat.completions.create(
        model="gpt-4-1106-preview",
        timeout=300,
        messages=[
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_prompt.format(code=outbound_code)},
        ],
    )

    print(res.usage)
    res = res.choices[0].message.content
    print(res)

    rs_path = os.path.join(dir_path, "outbounds.rs")
    with open(rs_path, "w") as f:
        f.write(res)

    pass
