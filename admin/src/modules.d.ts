// ra-language-german bringt keine Typen mit.
declare module "ra-language-german" {
  import type { TranslationMessages } from "react-admin";
  const messages: TranslationMessages;
  export default messages;
}
