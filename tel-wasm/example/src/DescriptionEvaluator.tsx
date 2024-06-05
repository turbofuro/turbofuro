import { useState, useEffect } from "react";
import { useDebounce } from "./utils";
import * as tel from "@turbofuro/tel-wasm";

export default function DescriptionEvaluator() {
  const [expression, setExpression] = useState("string.uuid | null");

  const debouncedExpression = useDebounce(expression);

  const [result, setResult] = useState<tel.DescriptionEvaluationResult>();
  const [parsed, setParsed] = useState<tel.DescriptionParseResult>();

  useEffect(() => {
    try {
      const parsed = tel.parseDescription(debouncedExpression);
      setParsed(parsed);

      const result = tel.evaluateDescription(debouncedExpression);
      setResult(result);

      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } catch (err: any) {
      console.error(err);
      setResult({
        value: {
          type: "error",
          error: {
            code: "PARSE_ERROR",
            errors: [],
          },
        },
      });
    }
  }, [debouncedExpression]);

  return (
    <>
      <nav>Description</nav>
      <div className="split">
        <div className="left">
          <h3>Expression:</h3>
          <textarea
            id="expression"
            value={expression}
            onChange={(e) => setExpression(e.target.value)}
          ></textarea>
        </div>
        <div className="right">
          {result?.value.type == "error" && (
            <div className="error">
              <span>Errored</span>
              <pre>{result.value.error.code}</pre>
            </div>
          )}
          {result?.value != null && (
            <pre id="output">{JSON.stringify(result.value, null, 2)}</pre>
          )}
          {result?.value != null && (
            <pre id="output">{tel.getNotation(result.value)}</pre>
          )}
          <pre className="text-sm">{JSON.stringify(parsed, null, 2)}</pre>
        </div>
      </div>
    </>
  );
}
