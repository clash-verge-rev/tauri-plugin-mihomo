import { json } from "@codemirror/lang-json";
import { invoke } from "@tauri-apps/api/core";
import CodeMirror from "@uiw/react-codemirror";
import { useCallback, useState } from "react";
import { getGroups } from "tauri-plugin-mihomo-api";
import "./App.css";

function App() {
  const [response, setResponse] = useState("");

  const format_json = useCallback(async (text: string) => {
    return await invoke<string>("cmd_format_json", { text });
  }, []);

  const check = useCallback(async () => {
    try {
      const data = await getGroups();
      const formattedJson = await format_json(JSON.stringify(data));
      setResponse(formattedJson);
    } catch (err: any) {
      setResponse(err.toString());
    }
  }, [format_json]);

  return (
    <main style={{ backgroundColor: "white" }}>
      <div className="row">
        <button type="button" onClick={() => check()}>
          Check
        </button>
      </div>
      <CodeMirror
        style={{ marginTop: "10px", textAlign: "left" }}
        width="100%"
        height="85dvh"
        minHeight="480px"
        value={response}
        theme={"dark"}
        extensions={[json()]}
      />
    </main>
  );
}

export default App;
