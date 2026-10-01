import { useState } from "react";
import type { GaugeUi } from "../api";

export default function GaugeChooser({ available, selected, customized, onChange }: {
  available: GaugeUi[];
  selected: GaugeUi[];
  customized: boolean;
  onChange: (names: string[] | null) => void;
}) {
  const [search, setSearch] = useState("");
  const names = selected.map((g) => g.name);
  const query = search.trim().toLowerCase();
  const move = (index: number, offset: number) => {
    const next = [...names];
    [next[index], next[index + offset]] = [next[index + offset], next[index]];
    onChange(next);
  };
  return (
    <details className="gauge-chooser">
      <summary>Choose gauges ({selected.length})</summary>
      <p>Changes are saved in this browser and override the INI dashboard selection.</p>
      <button onClick={() => onChange(null)} disabled={!customized}>Reset to defaults</button>
      {selected.length > 0 && <>
        <h3>Dashboard order</h3>
        <ol className="gauge-order">
          {selected.map((g, i) => <li key={g.name}>
            <span>{g.title} <small>{g.name}</small></span>
            <button disabled={i === 0} aria-label={`Move ${g.title} up`} onClick={() => move(i, -1)}>↑</button>
            <button disabled={i === selected.length - 1} aria-label={`Move ${g.title} down`} onClick={() => move(i, 1)}>↓</button>
          </li>)}
        </ol>
      </>}
      <h3>Available gauges</h3>
      <input type="search" aria-label="Search gauges" placeholder="Search gauges" value={search} onChange={(e) => setSearch(e.target.value)} />
      <div className="gauge-choices">
        {available.filter((g) => `${g.title} ${g.name} ${g.channel}`.toLowerCase().includes(query)).map((g) => (
          <label key={g.name}>
            <input type="checkbox" checked={names.includes(g.name)} onChange={(e) => onChange(e.target.checked ? [...names, g.name] : names.filter((name) => name !== g.name))} />
            <span>{g.title}{g.units && ` (${g.units})`}<small>{g.name}</small></span>
          </label>
        ))}
      </div>
    </details>
  );
}
