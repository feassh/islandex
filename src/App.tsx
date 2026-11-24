import { DynamicIsland } from "./components/DynamicIsland";
import { SettingsPanel } from "./components/SettingsPanel";
import "./App.css";

function isSettingsRoute(): boolean {
  if (typeof window === "undefined") return false;
  return window.location.hash.startsWith("#/settings");
}

function App() {
  if (isSettingsRoute()) {
    return <SettingsPanel />;
  }
  return <DynamicIsland />;
}

export default App;
