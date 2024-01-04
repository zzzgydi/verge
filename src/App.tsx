import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "./components/ui/button";

function App() {
  const [url, setUrl] = useState("");

  async function greet() {
    try {
      await invoke("import_profile", { req: { url } });
    } catch (err) {
      console.error(err);
    }
  }

  useEffect(() => {
    async function init() {
      try {
        const data = await invoke("list_profiles");
        console.log(data);
      } catch (err) {
        console.error(err);
      }
    }
    init();
  }, []);

  return (
    <div className="p-10">
      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault();
          greet();
        }}
      >
        <input
          id="greet-input"
          onChange={(e) => setUrl(e.currentTarget.value)}
          placeholder="Enter a name..."
        />
        <Button type="submit">Greet</Button>
      </form>
    </div>
  );
}

export default App;
