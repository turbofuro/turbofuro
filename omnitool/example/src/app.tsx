import { NavLink, Route, Routes } from "react-router";
import Welcome from "./welcome";
import DescriptionPlayground from "./description-playground";
import TelPlayground from "./tel-playground";
import AnalysisPlayground from "./analysis-playground";

export default function App() {
  return (
    <div>
      <nav>
        <input type="checkbox" id="nav-check" />
        <div className="nav-top">
          <label htmlFor="nav-check" id="nav-button">
            <svg
              xmlns="http://www.w3.org/2000/svg"
              width="30"
              height="30"
              fill="currentColor"
              className="bi bi-list"
              viewBox="0 0 16 16"
            >
              <path
                fill-rule="evenodd"
                d="M2.5 12a.5.5 0 0 1 .5-.5h10a.5.5 0 0 1 0 1H3a.5.5 0 0 1-.5-.5zm0-4a.5.5 0 0 1 .5-.5h10a.5.5 0 0 1 0 1H3a.5.5 0 0 1-.5-.5zm0-4a.5.5 0 0 1 .5-.5h10a.5.5 0 0 1 0 1H3a.5.5 0 0 1-.5-.5z"
              />
            </svg>
          </label>
          <div className="mx-lg row center gap-md">
            <img className="logo mr-sm" src="/logo_white.png" alt="turbofuro" />
            <div className="logo-text">turbofuro</div>
          </div>
        </div>
        <div className="nav-links">
          <NavLink className="nav-link" to="/analysis">
            Analysis
          </NavLink>
          <NavLink className="nav-link" to="/tel">
            TEL
          </NavLink>
          <NavLink className="nav-link" to="/description">
            Description
          </NavLink>
          <div className="spacer"></div>
          <a className="nav-link" href="https://github.com/turbofuro/turbofuro">
            GitHub
          </a>
        </div>
      </nav>
      <main>
        <Routes>
          <Route path="/" element={<Welcome />} />
          <Route path="/analysis" element={<AnalysisPlayground />} />
          <Route path="/tel" element={<TelPlayground />} />
          <Route path="/description" element={<DescriptionPlayground />} />
        </Routes>
      </main>
    </div>
  );
}
