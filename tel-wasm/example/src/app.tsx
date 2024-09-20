import { useState } from "react";
import DescriptionEvaluator from "./DescriptionEvaluator";
import Evaluator from "./Evaluator";

export default function App() {
  return (
    <>
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
          <div className="font-mono mh-md">TEL</div>
        </div>
        <div className="nav-links"></div>
      </nav>
      <Evaluator />
      <DescriptionEvaluator />
    </>
  );
}
