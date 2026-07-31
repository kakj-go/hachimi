export function forwardedCliArguments(arguments_) {
  return arguments_.filter((argument) => argument !== "--");
}

export function namedCliArguments(arguments_) {
  const forwarded = forwardedCliArguments(arguments_);
  if (forwarded.length % 2 !== 0) throw new Error("release_cli_arguments_invalid");
  const parsed = new Map();
  for (let index = 0; index < forwarded.length; index += 2) {
    const name = forwarded[index];
    if (!name.startsWith("--")) throw new Error(`release_cli_argument_name_invalid:${name}`);
    parsed.set(name, forwarded[index + 1]);
  }
  return parsed;
}
