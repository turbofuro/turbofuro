import { useState, useEffect } from "react";
import { useDebounce } from "./utils";
import * as omnitool from "@turbofuro/omnitool";
import { DEFAULT_DECLARATIONS, DEFAULT_INSTRUCTIONS } from "./defaults";

export default function Analysis() {
  const [instructionsJson, setInstructionsJson] = useState(
    JSON.stringify(DEFAULT_INSTRUCTIONS, null, 2)
  );
  const [declarationsJson, setDeclarationsJson] = useState(
    JSON.stringify(DEFAULT_DECLARATIONS, null, 2)
  );

  const debouncedInstructions = useDebounce(instructionsJson);
  const debouncedDeclarations = useDebounce(declarationsJson);

  const [result, setResult] = useState<any>();
  const [error, setError] = useState<string>();

  useEffect(() => {
    let instructions, declarations;

    setError(undefined);
    setResult(undefined);

    try {
      instructions = JSON.parse(debouncedInstructions);
    } catch (err) {
      setError("Instructions are not valid JSON");
      return;
    }

    try {
      declarations = JSON.parse(debouncedDeclarations);
    } catch (err) {
      setError("Declarations are not valid JSON");
      return;
    }

    try {
      const parsed = omnitool.analyze(instructions, declarations, []);
      setResult(parsed);

      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } catch (err: any) {
      console.error(err);
      setError(err.toString());
    }
  }, [debouncedDeclarations, debouncedInstructions]);

  return (
    <div className="w-full">
      <div className="split">
        <div className="left">
          <h4 className="m-md">Instructions</h4>
          <textarea
            id="instructions"
            value={instructionsJson}
            className="field w-full"
            onChange={(e) => setInstructionsJson(e.target.value)}
          ></textarea>
          <h4 className="m-md">Declarations</h4>
          <textarea
            id="declarations"
            className="field w-full"
            value={declarationsJson}
            onChange={(e) => setDeclarationsJson(e.target.value)}
          ></textarea>
        </div>
        <div className="right">
          <h4 className="m-md">Output</h4>
          {error && <pre className="error mb-md">{error}</pre>}
          <pre className="output">{JSON.stringify(result, null, 2)}</pre>
        </div>
      </div>
    </div>
  );
}
