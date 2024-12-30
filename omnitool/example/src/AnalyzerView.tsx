import { useState, useEffect } from "react";
import { useDebounce } from "./utils";
import * as omnitool from "@turbofuro/omnitool";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type StepAnalysis = any;

type Result =
  | {
      type: "error";
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      error: any;
    }
  | {
      type: "success";
      value: StepAnalysis;
    };

const DEFAULT_DECLARATIONS = [
  {
    tag: "fd1",
    id: "hF-T7oV7lIU6EsbDL8U7p",
    meta: {
      name: "Test",
      description: "",
      shortDescription: "",
      theme: "gray",
    },
    parameters: [
      {
        name: "path",
        description: "Path like /home/test/a.txt",
        optional: false,
        type: "tel",
        valueDescription: "string",
      },
    ],
    annotations: [],
    decorator: false,
  },
  {
    tag: "fd1",
    id: "d9yDX79zAbQmEbX9kGIGn",
    meta: {
      name: "Sum",
      description: "",
      shortDescription: "",
      theme: "gray",
    },
    parameters: [
      {
        name: "a",
        description: "",
        optional: false,
        type: "tel",
        valueDescription: "number",
      },
      {
        name: "b",
        description: "",
        optional: false,
        type: "tel",
        valueDescription: "number",
      },
    ],
    annotations: [],
    outputDescription: "number",
    decorator: false,
  },
  {
    id: "parse_json",
    tag: "fd1",
    meta: {
      icon: "https://assets.turboflow.dev/math-operations.svg",
      name: "Parse JSON",
      theme: "gray",
      description:
        'This function parses a JSON string and returns an equivalent storage value.\n\n##### Example 1:\nParse a stringified JSON object\n```furo\n[\n  {\n    "id": "sPtxHVHIdnw1q5JJUoMyg",\n    "type": "call",\n    "callee": "local/parse_json",\n    "parameters": [\n      {\n        "type": "tel",\n        "name": "json",\n        "expression": "\\"{ \\\\\\"message\\\\\\": \\\\\\"Hello World\\\\\\" }\\""\n      }\n    ],\n    "storeAs": "parsed"\n  }\n]\n```\nwill store following object as `parsed`: \n```\n{\n  message: "Hello World"\n}\n```',
      shortDescription:
        'This function parses a JSON string and returns an equivalent storage value.\n\n##### Example 1:\nParse a stringified JSON object\n```furo\n[\n  {\n    "id": "sPtxHVHIdnw1q5JJUoMyg",\n    "type": "call",\n    "callee": "local/parse_json",\n    "parameters": [\n      {\n        "type": "tel",\n        "name": "json",\n        "expression": "\\"{ \\\\\\"message\\\\\\": \\\\\\"Hello World\\\\\\" }\\""\n      }\n    ],\n    "storeAs": "parsed"\n  }\n]\n```\nwill store following object as `parsed`: \n```\n{\n  message: "Hello World"\n}\n```',
    },
    parameters: [
      {
        name: "json",
        type: "tel",
        optional: false,
        description: "JSON string",
        valueDescription: "string.json",
      },
    ],
    annotations: [
      {
        type: "exported",
      },
    ],
    moduleVersionId: "30157108-d8d1-43ac-ba39-e82f0061dd0e",
    outputDescription: "any",
  },
  {
    id: "to_json",
    tag: "fd1",
    meta: {
      icon: "https://assets.turboflow.dev/math-operations.svg",
      name: "Convert to JSON",
      theme: "gray",
      description:
        'This function performs serialization of a storage value to JSON format string. \n\n##### Example 1:\nConvert object to JSON string.\n```furo\n[\n  {\n    "id": "5QX098PQbGRWRdNBNdEUu",\n    "type": "call",\n    "callee": "local/to_json",\n    "parameters": [\n      {\n        "type": "tel",\n        "name": "value",\n        "expression": "{\\n  message: \\"Hello World\\"\\n}"\n      }\n    ],\n    "storeAs": "json"\n  }\n]\n```\nwill store following string (raw data) as `json`: \n```\n{"message":"Hello World"}}\n```\n\n##### Example 2:\nConvert object to JSON string with pretty spacing.\n```furo\n[\n  {\n    "id": "XoalSW-MH0lwbU-IxGmWA",\n    "type": "call",\n    "callee": "local/to_json",\n    "parameters": [\n      {\n        "type": "tel",\n        "name": "value",\n        "expression": "{\\n  message: \\"Hello World\\",\\n  details: {\\n    status: 200\\n  }\\n}"\n      },\n      {\n        "type": "tel",\n        "name": "pretty",\n        "expression": "true"\n      }\n    ],\n    "storeAs": "json"\n  }\n]\n```\nwill store following string (raw data) as `json`: \n```\n{\n  "message": "Hello World",\n  "details": {\n    "status": 200\n  }\n}\n```',
      shortDescription:
        'This function performs serialization of a storage value to JSON format string. \n\n##### Example 1:\nConvert object to JSON string.\n```furo\n[\n  {\n    "id": "5QX098PQbGRWRdNBNdEUu",\n    "type": "call",\n    "callee": "local/to_json",\n    "parameters": [\n      {\n        "type": "tel",\n        "name": "value",\n        "expression": "{\\n  message: \\"Hello World\\"\\n}"\n      }\n    ],\n    "storeAs": "json"\n  }\n]\n```\nwill store following string (raw data) as `json`: \n```\n{"message":"Hello World"}}\n```\n\n##### Example 2:\nConvert object to JSON string with pretty spacing.\n```furo\n[\n  {\n    "id": "XoalSW-MH0lwbU-IxGmWA",\n    "type": "call",\n    "callee": "local/to_json",\n    "parameters": [\n      {\n        "type": "tel",\n        "name": "value",\n        "expression": "{\\n  message: \\"Hello World\\",\\n  details: {\\n    status: 200\\n  }\\n}"\n      },\n      {\n        "type": "tel",\n        "name": "pretty",\n        "expression": "true"\n      }\n    ],\n    "storeAs": "json"\n  }\n]\n```\nwill store following string (raw data) as `json`: \n```\n{\n  "message": "Hello World",\n  "details": {\n    "status": 200\n  }\n}\n```',
    },
    parameters: [
      {
        name: "value",
        type: "tel",
        optional: false,
        description: "Value to convert to JSON",
        valueDescription: "any",
      },
      {
        name: "pretty",
        type: "tel",
        optional: true,
        description: "Whether to use pretty formatting",
        valueDescription: "boolean",
      },
    ],
    annotations: [
      {
        type: "exported",
      },
    ],
    moduleVersionId: "30157108-d8d1-43ac-ba39-e82f0061dd0e",
    outputDescription: "string.json",
  },
  {
    id: "parse_urlencoded",
    tag: "fd1",
    meta: {
      icon: "https://assets.turboflow.dev/math-operations.svg",
      name: "Parse URL encoded",
      theme: "gray",
      description:
        'This function performs deserialization of URI encoding string.\n\nExample 1:\nIt takes string like\n```\nname=Olaf&age=25&city=Hamburg\n```\nconverts to\n```\n{\n    name: "Olaf",\n    age: "25",\n    city: "Hamburg"\n}\n```\n\nExample 2:\nIt takes string like\n```\ndata=%20%3C%3E%3F%23%5B%5D%40%21%24%26%27%28%29%2A%2B%2C%3B%3D%3A%40%25\n```\nconverts to storage value\n```\n{\n    data: " <>?#[]@!$&\'()*+,;=:@%"\n}\n```\nNote: That leading space is because 20 is a hex representation of a space character.\n\nSee more on: [RFC3986 Section 2.1 Percent-Encoding](https://www.rfc-editor.org/rfc/rfc3986#section-2.1)',
      shortDescription:
        'This function performs deserialization of URI encoding string.\n\nExample 1:\nIt takes string like\n```\nname=Olaf&age=25&city=Hamburg\n```\nconverts to\n```\n{\n    name: "Olaf",\n    age: "25",\n    city: "Hamburg"\n}\n```\n\nExample 2:\nIt takes string like\n```\ndata=%20%3C%3E%3F%23%5B%5D%40%21%24%26%27%28%29%2A%2B%2C%3B%3D%3A%40%25\n```\nconverts to storage value\n```\n{\n    data: " <>?#[]@!$&\'()*+,;=:@%"\n}\n```\nNote: That leading space is because 20 is a hex representation of a space character.\n\nSee more on: [RFC3986 Section 2.1 Percent-Encoding](https://www.rfc-editor.org/rfc/rfc3986#section-2.1)',
    },
    parameters: [
      {
        name: "urlencoded",
        type: "tel",
        optional: false,
        description: "URL encoded string",
        valueDescription: "string",
      },
    ],
    annotations: [
      {
        type: "exported",
      },
    ],
    moduleVersionId: "30157108-d8d1-43ac-ba39-e82f0061dd0e",
    outputDescription: "object.record",
  },
  {
    id: "to_urlencoded",
    tag: "fd1",
    meta: {
      icon: "https://assets.turboflow.dev/math-operations.svg",
      name: "Convert to URL encoded",
      theme: "gray",
      description:
        'Converts storage value to URL encoding\n\nExample 1:\nIt takes storage value like\n```\n{\n    name: "Olaf",\n    age: "25",\n    city: "Hamburg"\n}\n```\nconverts to URI-encoded string\n```\nname=Olaf&age=25&city=Hamburg\n```\n\nExample 2:\nIt takes storage value like\n```\n{\n    data: " <>?#[]@!$&\'()*+,;=:@%"\n}\n```\nconverts to URI-encoded string\n```\ndata=%20%3C%3E%3F%23%5B%5D%40%21%24%26%27%28%29%2A%2B%2C%3B%3D%3A%40%25\n```',
      shortDescription:
        'Converts storage value to URL encoding\n\nExample 1:\nIt takes storage value like\n```\n{\n    name: "Olaf",\n    age: "25",\n    city: "Hamburg"\n}\n```\nconverts to URI-encoded string\n```\nname=Olaf&age=25&city=Hamburg\n```\n\nExample 2:\nIt takes storage value like\n```\n{\n    data: " <>?#[]@!$&\'()*+,;=:@%"\n}\n```\nconverts to URI-encoded string\n```\ndata=%20%3C%3E%3F%23%5B%5D%40%21%24%26%27%28%29%2A%2B%2C%3B%3D%3A%40%25\n```',
    },
    parameters: [
      {
        name: "value",
        type: "tel",
        optional: false,
        description: "Value to convert to URL encoded",
        valueDescription: "object.record",
      },
    ],
    annotations: [
      {
        type: "exported",
      },
    ],
    moduleVersionId: "30157108-d8d1-43ac-ba39-e82f0061dd0e",
    outputDescription: "string",
  },
  {
    id: "QkXeMcyG-l0LqQCF9zwCx",
    tag: "fd1",
    meta: {
      icon: "https://assets.turboflow.dev/math-operations.svg",
      name: "Parse URL",
      theme: "gray",
      description:
        'This function performs deserialization of URL string.\n\nExample 1:\nIt takes string like\n```furo\n[\n  {\n    "id": "SzLbqsdO7BzVrS0K6rot3",\n    "type": "call",\n    "callee": "local/QkXeMcyG-l0LqQCF9zwCx",\n    "parameters": [\n      {\n        "type": "tel",\n        "name": "url",\n        "expression": "\\"https://api.turbofuro.com/test?param_one=hello&param_two=world#fragment\\"",\n        "valueDescription": "string"\n      }\n    ],\n    "storeAs": "result"\n  }\n]\n```\nconverts to a URL object storage value\n```\n{\n  fragment: "fragment",\n  host: "api.turbofuro.com",\n  origin: "https://api.turbofuro.com",\n  password: null,\n  path: "/test",\n  port: null,\n  query: "param_one=hello&param_two=world",\n  scheme: "https",\n  username: ""\n}\n```\n\nExample 2:\nIt takes string like\n```furo\n[\n  {\n    "id": "6rqH2MwaLarzBU0qBiaGc",\n    "type": "call",\n    "callee": "local/QkXeMcyG-l0LqQCF9zwCx",\n    "parameters": [\n      {\n        "type": "tel",\n        "name": "url",\n        "expression": "\\"postgres://username:password@localhost:5432/database\\"",\n        "valueDescription": "string"\n      }\n    ],\n    "storeAs": "result"\n  }\n]\n```\nconverts to a URL object storage value\n```\n{\n  fragment: null,\n  host: "localhost",\n  origin: "null",\n  password: "password",\n  path: "/database",\n  port: 5432,\n  query: null,\n  scheme: "postgres",\n  username: "username"\n}\n```\nNote: That leading space is because 20 is a hex representation of a space character.\n\nSee more on: [RFC3986 Section 2.1 Percent-Encoding](https://www.rfc-editor.org/rfc/rfc3986#section-2.1)',
      shortDescription:
        'This function performs deserialization of URL string.\n\nExample 1:\nIt takes string like\n```furo\n[\n  {\n    "id": "SzLbqsdO7BzVrS0K6rot3",\n    "type": "call",\n    "callee": "local/QkXeMcyG-l0LqQCF9zwCx",\n    "parameters": [\n      {\n        "type": "tel",\n        "name": "url",\n        "expression": "\\"https://api.turbofuro.com/test?param_one=hello&param_two=world#fragment\\"",\n        "valueDescription": "string"\n      }\n    ],\n    "storeAs": "result"\n  }\n]\n```\nconverts to a URL object storage value\n```\n{\n  fragment: "fragment",\n  host: "api.turbofuro.com",\n  origin: "https://api.turbofuro.com",\n  password: null,\n  path: "/test",\n  port: null,\n  query: "param_one=hello&param_two=world",\n  scheme: "https",\n  username: ""\n}\n```\n\nExample 2:\nIt takes string like\n```furo\n[\n  {\n    "id": "6rqH2MwaLarzBU0qBiaGc",\n    "type": "call",\n    "callee": "local/QkXeMcyG-l0LqQCF9zwCx",\n    "parameters": [\n      {\n        "type": "tel",\n        "name": "url",\n        "expression": "\\"postgres://username:password@localhost:5432/database\\"",\n        "valueDescription": "string"\n      }\n    ],\n    "storeAs": "result"\n  }\n]\n```\nconverts to a URL object storage value\n```\n{\n  fragment: null,\n  host: "localhost",\n  origin: "null",\n  password: "password",\n  path: "/database",\n  port: 5432,\n  query: null,\n  scheme: "postgres",\n  username: "username"\n}\n```\nNote: That leading space is because 20 is a hex representation of a space character.\n\nSee more on: [RFC3986 Section 2.1 Percent-Encoding](https://www.rfc-editor.org/rfc/rfc3986#section-2.1)',
    },
    parameters: [
      {
        name: "url",
        type: "tel",
        optional: false,
        description: "URL string",
        valueDescription: "string",
      },
    ],
    annotations: [
      {
        type: "exported",
      },
    ],
    moduleVersionId: "30157108-d8d1-43ac-ba39-e82f0061dd0e",
  },
  {
    id: "cMpMs1Qkl6snZM26mpDoe",
    tag: "fd1",
    meta: {
      icon: "https://assets.turboflow.dev/math-operations.svg",
      name: "Convert to URL",
      theme: "gray",
      description:
        'Converts storage value to URL\n\nExample 1:\nIt takes storage value like\n```furo\n[\n  {\n    "id": "Dk_sP7nmZw7X2gdo7WggF",\n    "type": "call",\n    "callee": "local/cMpMs1Qkl6snZM26mpDoe",\n    "parameters": [\n      {\n        "type": "tel",\n        "name": "host",\n        "expression": "\\"localhost\\"",\n        "valueDescription": "string"\n      },\n      {\n        "type": "tel",\n        "name": "scheme",\n        "expression": "\\"postgres\\"",\n        "valueDescription": "string"\n      },\n      {\n        "type": "tel",\n        "name": "port",\n        "expression": "5432",\n        "valueDescription": "number | string"\n      },\n      {\n        "type": "tel",\n        "name": "path",\n        "expression": "\\"/database\\"",\n        "valueDescription": "string"\n      },\n      {\n        "type": "tel",\n        "name": "username",\n        "expression": "\\"username\\"",\n        "valueDescription": "string"\n      },\n      {\n        "type": "tel",\n        "name": "password",\n        "expression": "\\"password\\"",\n        "valueDescription": "string"\n      }\n    ],\n    "storeAs": "url"\n  }\n]\n```\nconverts to URL string\n```\npostgres://username:password@localhost:5432/database\n```\n\nExample 2:\nIt takes storage value like\n```furo\n[\n  {\n    "id": "QsfTiB6EuMmWqj89rafH2",\n    "type": "call",\n    "callee": "local/cMpMs1Qkl6snZM26mpDoe",\n    "storeAs": "url",\n    "parameters": [\n      {\n        "type": "tel",\n        "name": "host",\n        "expression": "\\"api.turbofuro.com\\"",\n        "valueDescription": "string"\n      },\n      {\n        "type": "tel",\n        "name": "scheme",\n        "expression": "\\"https\\"",\n        "valueDescription": "string"\n      },\n      {\n        "type": "tel",\n        "name": "path",\n        "expression": "\\"/test\\"",\n        "valueDescription": "string"\n      },\n      {\n        "type": "tel",\n        "name": "query",\n        "expression": "\\"param_one=hello&param_two=world\\"",\n        "valueDescription": "string"\n      },\n      {\n        "type": "tel",\n        "name": "fragment",\n        "expression": "\\"fragment\\"",\n        "valueDescription": "string"\n      }\n    ]\n  }\n]\n```\nconverts to URL string\n```\nhttps://api.turbofuro.com/test?param_one=hello&param_two=world#fragment\n```',
      shortDescription:
        'Converts storage value to URL\n\nExample 1:\nIt takes storage value like\n```furo\n[\n  {\n    "id": "Dk_sP7nmZw7X2gdo7WggF",\n    "type": "call",\n    "callee": "local/cMpMs1Qkl6snZM26mpDoe",\n    "parameters": [\n      {\n        "type": "tel",\n        "name": "host",\n        "expression": "\\"localhost\\"",\n        "valueDescription": "string"\n      },\n      {\n        "type": "tel",\n        "name": "scheme",\n        "expression": "\\"postgres\\"",\n        "valueDescription": "string"\n      },\n      {\n        "type": "tel",\n        "name": "port",\n        "expression": "5432",\n        "valueDescription": "number | string"\n      },\n      {\n        "type": "tel",\n        "name": "path",\n        "expression": "\\"/database\\"",\n        "valueDescription": "string"\n      },\n      {\n        "type": "tel",\n        "name": "username",\n        "expression": "\\"username\\"",\n        "valueDescription": "string"\n      },\n      {\n        "type": "tel",\n        "name": "password",\n        "expression": "\\"password\\"",\n        "valueDescription": "string"\n      }\n    ],\n    "storeAs": "url"\n  }\n]\n```\nconverts to URL string\n```\npostgres://username:password@localhost:5432/database\n```\n\nExample 2:\nIt takes storage value like\n```furo\n[\n  {\n    "id": "QsfTiB6EuMmWqj89rafH2",\n    "type": "call",\n    "callee": "local/cMpMs1Qkl6snZM26mpDoe",\n    "storeAs": "url",\n    "parameters": [\n      {\n        "type": "tel",\n        "name": "host",\n        "expression": "\\"api.turbofuro.com\\"",\n        "valueDescription": "string"\n      },\n      {\n        "type": "tel",\n        "name": "scheme",\n        "expression": "\\"https\\"",\n        "valueDescription": "string"\n      },\n      {\n        "type": "tel",\n        "name": "path",\n        "expression": "\\"/test\\"",\n        "valueDescription": "string"\n      },\n      {\n        "type": "tel",\n        "name": "query",\n        "expression": "\\"param_one=hello&param_two=world\\"",\n        "valueDescription": "string"\n      },\n      {\n        "type": "tel",\n        "name": "fragment",\n        "expression": "\\"fragment\\"",\n        "valueDescription": "string"\n      }\n    ]\n  }\n]\n```\nconverts to URL string\n```\nhttps://api.turbofuro.com/test?param_one=hello&param_two=world#fragment\n```',
    },
    parameters: [
      {
        name: "host",
        type: "tel",
        optional: false,
        description: "Host",
        valueDescription: "string",
      },
      {
        name: "scheme",
        type: "tel",
        optional: true,
        description: "Scheme",
        valueDescription: "string",
      },
      {
        name: "port",
        type: "tel",
        optional: true,
        description: "Port",
        valueDescription: "number | string",
      },
      {
        name: "path",
        type: "tel",
        optional: true,
        description: "Path part of the URL",
        valueDescription: "string",
      },
      {
        name: "query",
        type: "tel",
        optional: true,
        description:
          "Query part of the URL. Note that this must be already a proper query, not an object or array.",
        valueDescription: "string",
      },
      {
        name: "fragment",
        type: "tel",
        optional: true,
        description: "Fragment is the part after #",
        valueDescription: "string",
      },
      {
        name: "username",
        type: "tel",
        optional: true,
        description: "Username for credetials",
        valueDescription: "string",
      },
      {
        name: "password",
        type: "tel",
        optional: true,
        description: "Password for credetials",
        valueDescription: "string",
      },
    ],
    annotations: [
      {
        type: "exported",
      },
    ],
    moduleVersionId: "30157108-d8d1-43ac-ba39-e82f0061dd0e",
  },
];

const DEFAULT_INSTRUCTIONS = [
  {
    id: "hF-T7oV7lIU6EsbDL8U7p",
    type: "defineFunction",
    name: "Test",
    description: "",
    parameters: [
      {
        name: "path",
        description: "Path like /home/test/a.txt",
        optional: false,
        type: "tel",
        valueDescription: "string",
      },
    ],
    body: [
      {
        id: "ctd1bYQ7Vf0DQOtrlginL",
        type: "assign",
        value: "path.trim()",
        to: "trimmed",
      },
      {
        id: "_Pw_v0PnW36ta_c3qIX7Z",
        type: "call",
        callee: "import/4e8cdd5a-bb8b-4b26-b274-b4ea299b90c4/parse_json",
        parameters: [
          {
            type: "tel",
            name: "json",
            expression:
              '```json\n{\n  message: "This should be a string!"\n}\n```',
          },
        ],
        storeAs: "result",
      },
      {
        id: "pRnjL_zkHBI5N2WjIfQQX",
        type: "if",
        condition: 'result.message == "Hello World"',
        then: [
          {
            id: "iDRSlzawrK_pCwssoRaFN",
            type: "return",
          },
        ],
        branches: [],
        else: [],
      },
      {
        id: "pqZA2VP1tcTIWazdHMaVq",
        type: "forEach",
        item: "item",
        items: "[1, 2, 3]",
        body: [
          {
            id: "Q38Stvyu-xBoVh5RzjGjN",
            type: "call",
            callee: "local/d9yDX79zAbQmEbX9kGIGn",
            parameters: [
              {
                type: "tel",
                name: "a",
                expression: "item",
              },
              {
                type: "tel",
                name: "b",
                expression: "5",
              },
            ],
          },
        ],
      },
      {
        id: "QrI_n3HQuk-LXcGD_u3yz",
        type: "assign",
        value: "hello",
        to: "undef",
      },
      {
        id: "MidGYy761QsSfrnfIhYTw",
        type: "return",
      },
      {
        id: "indx7bOYBOfBzQHuUb5uw",
        type: "assign",
        value: "4",
        to: "x",
      },
    ],
    exported: false,
    annotations: [],
    decorator: false,
  },
  {
    id: "d9yDX79zAbQmEbX9kGIGn",
    type: "defineFunction",
    name: "Sum",
    description: "",
    parameters: [
      {
        name: "a",
        description: "",
        optional: false,
        type: "tel",
        valueDescription: "number",
      },
      {
        name: "b",
        description: "",
        optional: false,
        type: "tel",
        valueDescription: "number",
      },
    ],
    body: [
      {
        id: "GxpMh7_Yz2KF2l-BXn7sd",
        type: "return",
        value: "a + b",
      },
    ],
    exported: false,
    annotations: [],
    outputDescription: "number",
    decorator: false,
  },
];

export default function AnalyzerView() {
  const [instructionsJson, setInstructionsJson] = useState(
    JSON.stringify(DEFAULT_INSTRUCTIONS, null, 2)
  );
  const [declarationsJson, setDeclarationsJson] = useState(
    JSON.stringify(DEFAULT_DECLARATIONS, null, 2)
  );

  const debouncedInstructions = useDebounce(instructionsJson);
  const debouncedDeclarations = useDebounce(declarationsJson);

  const [result, setResult] = useState<Result>();

  useEffect(() => {
    let instructions, declarations;
    try {
      instructions = JSON.parse(debouncedInstructions);
    } catch (err) {
      setResult({
        type: "error",
        error: {
          code: "INVALID_INSTRUCTIONS",
          errors: [],
        },
      });
      return;
    }

    try {
      declarations = JSON.parse(debouncedDeclarations);
    } catch (err) {
      setResult({
        type: "error",
        error: {
          code: "INVALID_DECLARATIONS",
          errors: [],
        },
      });
      return;
    }

    try {
      const parsed = omnitool.analyze(instructions, declarations, []);
      setResult({
        type: "success",
        value: parsed,
      });

      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } catch (err: any) {
      console.error(err);
      setResult({
        type: "error",
        error: {
          code: "ANALYZE_ERROR",
          errors: [],
        },
      });
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
          {result?.type == "error" && (
            <div className="error">
              <span>Errored</span>
              <pre>{result.error.code}</pre>
              <pre className="text-sm">
                {JSON.stringify(result.error.errors, null, 2)}
              </pre>
            </div>
          )}
          {result?.type == "success" && (
            <pre id="output">{JSON.stringify(result.value, null, 2)}</pre>
          )}
        </div>
      </div>
    </div>
  );
}
