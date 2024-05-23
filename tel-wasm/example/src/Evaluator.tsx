import { useState, useEffect } from "react";
import { useDebounce } from "./utils";
import * as tel from "@turbofuro/tel-wasm";

export default function Evaluator() {
  const [expression, setExpression] = useState("2 + 2");
  const [storage, setStorage] = useState("{}");
  const [environment, setEnvironment] = useState("{}");

  const debouncedExpression = useDebounce(expression);
  const debouncedStorage = useDebounce(storage);
  const debouncedEnvironment = useDebounce(environment);

  const [environmentError, setEnvironmentError] = useState<string>();
  const [storageError, setStorageError] = useState<string>();

  const [result, setResult] = useState<tel.EvaluationResult>();
  const [parsed, setParsed] = useState<tel.ParseResult>();

  useEffect(() => {
    try {
      JSON.parse(debouncedStorage);
      setStorageError(undefined);
    } catch (err) {
      setStorageError(String(err));
    }
  }, [debouncedStorage]);

  useEffect(() => {
    try {
      JSON.parse(debouncedEnvironment);
      setEnvironmentError(undefined);
    } catch (err) {
      setEnvironmentError(String(err));
    }
  }, [debouncedEnvironment]);

  useEffect(() => {
    try {
      const parsed = tel.parse(debouncedExpression);
      setParsed(parsed);

      const result = tel.evaluateValue(
        debouncedExpression,
        JSON.parse(debouncedStorage),
        JSON.parse(debouncedEnvironment)
      );

      setResult(result);
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } catch (err: any) {
      console.error(err);
      setResult({
        type: "error",
        error: {
          code: "PARSE_ERROR",
          errors: [],
        },
      });
    }
  }, [debouncedExpression, debouncedStorage, debouncedEnvironment]);

  return (
    <>
      <nav>TEL</nav>
      <div className="split">
        <div className="left">
          <h3>Expression:</h3>
          <textarea
            id="expression"
            value={expression}
            onChange={(e) => setExpression(e.target.value)}
          ></textarea>
          <h3>Storage:</h3>
          <textarea
            id="storage"
            value={storage}
            onChange={(e) => setStorage(e.target.value)}
          ></textarea>
          <button
            disabled={storageError != null}
            onClick={() => {
              setStorage(JSON.stringify(JSON.parse(storage), null, 2));
            }}
          >
            Format
          </button>
          {storageError && <span className="error">{storageError}</span>}
          <h3>Environment:</h3>
          <textarea
            id="environment"
            value={environment}
            onChange={(e) => setEnvironment(e.target.value)}
          ></textarea>
          <button
            disabled={environmentError != null}
            onClick={() => {
              setStorage(JSON.stringify(JSON.parse(environment), null, 2));
            }}
          >
            Format
          </button>
          {environmentError && (
            <span className="error">{environmentError}</span>
          )}
        </div>
        <div className="right">
          {result?.type == "error" && (
            <div className="error">
              <span>Errored</span>
              <pre>{result.error.code}</pre>
            </div>
          )}
          {result?.type == "success" && (
            <pre id="output">{JSON.stringify(result.value, null, 2)}</pre>
          )}
          <pre className="text-sm">{JSON.stringify(parsed, null, 2)}</pre>
        </div>
      </div>
    </>
  );
}
