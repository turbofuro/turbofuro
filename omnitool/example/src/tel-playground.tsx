import { useState, useEffect } from "react";
import { useDebounce } from "./utils";
import * as tel from "@turbofuro/omnitool";

export default function TelPlayground() {
  const [expression, setExpression] = useState("2 + 2");
  const [storage, setStorage] = useState("{}");
  const [environment, setEnvironment] = useState("{}");

  const debouncedExpression = useDebounce(expression);
  const debouncedStorage = useDebounce(storage);
  const debouncedEnvironment = useDebounce(environment);

  const [environmentError, setEnvironmentError] = useState<string>();
  const [storageError, setStorageError] = useState<string>();

  const [result, setResult] = useState<tel.EvaluationResult | {}>();
  const [parsed, setParsed] = useState<tel.ParseResult>();
  const [error, setError] = useState<string>();

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
      setError(undefined);
      setParsed(undefined);

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
      setError(err.toString());
    }
  }, [debouncedExpression, debouncedStorage, debouncedEnvironment]);

  return (
    <>
      <div className="split">
        <div className="left">
          <h4 className="m-md">Expression</h4>
          <textarea
            id="expression"
            value={expression}
            className="field w-full"
            onChange={(e) => setExpression(e.target.value)}
          ></textarea>
          <h4 className="m-md">Storage</h4>
          <textarea
            id="storage"
            value={storage}
            className="field w-full"
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
          <h4 className="m-md">Environment</h4>
          <textarea
            id="environment"
            value={environment}
            className="field w-full"
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
          <h4 className="m-md">Output</h4>
          <pre className="output mb-md">{JSON.stringify(result, null, 2)}</pre>
          {error && <pre className="error mb-md">{error}</pre>}
          <pre className="text-sm">{JSON.stringify(parsed, null, 2)}</pre>
        </div>
      </div>
    </>
  );
}
