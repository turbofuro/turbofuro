// Storage
// {
//     "request": {
//         "headers": {
//             "access-control-allow-origin": "*",
//             "user-agent": "Tel",
//         },
//         "body": {
//             "id": 1,
//             "name": "John Doe"
//         }
//     },
//     "numbers": [1, 2, 3, 4, 5],
// }
// Environment
// {
//     "REDIS_URL": "redis://localhost:6379",
//     "TOKEN": "test",
//     "NUMBER": 3,
// }
5
number
"Hello World"
string
[1, 2, 3]
array
{ a: 5 }
object
true
boolean
null
any
null
null
null
string | null
5
5 | 10
"hello" | "world"
"hello" | "world" | "there"
[1, "hello", true]
[number, string, boolean]
{ a: 4 }
object
{ a: 5, b: 4}
{ a: number }
{ a: { b: "hello" } }
{ a: { b: string } }
{ a: { b: null } }
{ a: { b: string | null } }
{ a: 4 }
{ b: string | null }
"system://images"
string | null
{ icon: "system://images" }
{ icon: string | null }
{ icon: "system://images", label: string, value: null }
{ description: string | null, icon: string | null, label: string, theme: string | null, value: any }
{ icon: "system://images", label: string, value: null }[]
{ description: string | null, icon: string | null, label: string, theme: string | null, value: any }[] 
[{ a: "Hello", b: 5 }, { a: null, b: 6 }]
{ a: string | null, b: number }[]