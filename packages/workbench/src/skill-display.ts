import type { SkillRecord } from "@hachimi/contracts";

const BUILTIN_NAMES: Record<string, [string, string]> = {
  "office documents": ["Word", "Word"],
  documents: ["Word", "Word"],
  "office pdf": ["PDF", "PDF"],
  pdf: ["PDF", "PDF"],
  "office presentations": ["PowerPoint", "PowerPoint"],
  presentations: ["PowerPoint", "PowerPoint"],
  "office spreadsheets": ["Excel", "Excel"],
  spreadsheets: ["Excel", "Excel"],
  "office file organizer": ["文件整理", "File organizer"],
  "file organizer": ["文件整理", "File organizer"],
  "find-skills": ["技能发现", "Skill discovery"],
};

export function skillDisplayName(skill: SkillRecord, zh: boolean): string {
  if (skill.scope === "built_in") {
    const candidates = [skill.name, skill.qualifiedName, skill.interface?.displayName]
      .filter((value): value is string => Boolean(value))
      .map((value) => value.toLowerCase());
    for (const candidate of candidates) {
      const concise = BUILTIN_NAMES[candidate];
      if (concise) return concise[zh ? 0 : 1];
    }
  }
  return skill.interface?.displayName?.trim() || skill.qualifiedName || skill.name;
}
