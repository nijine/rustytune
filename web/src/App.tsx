import { useEffect, useState } from "react";

interface Health {
  name: string;
  version: string;
}

export default function App() {
  const [health, setHealth] = useState<Health | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetch("/api/health")
      .then((r) => r.json())
      .then(setHealth)
      .catch((e) => setError(String(e)));
  }, []);

  return (
    <main>
      <h1>rustytune</h1>
      <p className="tagline">open tuning for Speeduino</p>
      {health && (
        <p className="status ok">
          server connected — v{health.version}
        </p>
      )}
      {error && <p className="status err">server unreachable: {error}</p>}
      {!health && !error && <p className="status">connecting…</p>}
    </main>
  );
}
