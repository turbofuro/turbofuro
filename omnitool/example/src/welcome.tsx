export default function Welcome() {
  return (
    <div className="container">
      <h2>Welcome to Omnitool Playground</h2>
      <p>
        This small web application lets you play with Omnitool. You can use it
        to preview the output of various Omnitool functions. Start by selecting
        one of the subsections in the navigation menu.
      </p>
      <p>Fields apart of expressions are JSON formatted.</p>
      <img src="/catloaf.png" width={140} alt="catloaf" />
    </div>
  );
}
