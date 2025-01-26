import { useState, useEffect } from "react";
import { useDebounce } from "./utils";
import * as tel from "@turbofuro/omnitool";

export default function DescriptionPlayground() {
  const [expression, setExpression] = useState("string.uuid | null");

  const debouncedExpression = useDebounce(expression);

  const [result, setResult] = useState<tel.DescriptionEvaluationResult>();
  const [parsed, setParsed] = useState<tel.DescriptionParseResult>();

  const [error, setError] = useState<string>();

  useEffect(() => {
    try {
      setError(undefined);
      setResult(undefined);

      const parsed = tel.parseDescription(debouncedExpression);
      setParsed(parsed);

      const result = tel.evaluateDescription(debouncedExpression);
      setResult(result);

      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } catch (err: any) {
      console.error(err);
      setError(err.toString());
    }
  }, [debouncedExpression]);

  return (
    <div className="w-full">
      <div className="split">
        <div className="left">
          <h4 className="m-md">Notation</h4>
          <textarea
            id="expression"
            className="field w-full"
            value={expression}
            onChange={(e) => setExpression(e.target.value)}
          ></textarea>
        </div>
        <div className="right">
          <h4 className="m-md">Output</h4>
          <pre className="output mb-md">{JSON.stringify(result, null, 2)}</pre>
          {error && <pre className="error mb-md">{error}</pre>}
          <pre className="text-sm">{JSON.stringify(parsed, null, 2)}</pre>
        </div>
      </div>
    </div>
  );
}
