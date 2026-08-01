import { json } from "@codemirror/lang-json";
import { invoke } from "@tauri-apps/api/core";
import CodeMirror from "@uiw/react-codemirror";
import { useCallback, useEffect, useRef, useState } from "react";
import { getGroups, MihomoWebSocket } from "tauri-plugin-mihomo-api";

import "./App.css";

function App() {
  const [response, setResponse] = useState("");
  const wsRef = useRef<MihomoWebSocket[]>([]);

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

  const connectMihomoConnsWs = useCallback(async () => {
    try {
      const ws = await MihomoWebSocket.connect_connections();
      console.log(ws.id, typeof ws.id);
      ws.addListener((msg) => {
        console.log(msg);
      });
      wsRef.current.push(ws);
    } catch (err: any) {
      setResponse(err.toString());
    }
  }, []);

  const closeMihomoConnsWs = useCallback(async () => {
    try {
      const ws = wsRef.current?.pop();
      console.log(ws?.id);
      ws?.close();
    } catch (err: any) {
      setResponse(err.toString());
    }
  }, []);

  useEffect(() => {
    const interval = setInterval(() => {
      MihomoWebSocket.get_all_instances().then((instances) => {
        instances.forEach((instance) => {
          console.log(instance);
        });
      });
    }, 1000);
    return () => clearInterval(interval);
  }, []);

  return (
    <main style={{ backgroundColor: "white" }}>
      <div className="row">
        <button type="button" onClick={() => check()}>
          Check
        </button>
        <button type="button" onClick={() => connectMihomoConnsWs()}>
          Connect Mihomo Connections
        </button>
        <button type="button" onClick={() => closeMihomoConnsWs()}>
          Disconnect Mihomo Connections
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
