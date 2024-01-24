// In these cases we are saving "hello" to storage
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
data CHECK data == "hello"
request["headers"] CHECK request["headers"] == "hello"
request["headers"]["user-agent"] CHECK request["headers"]["user-agent"] == "hello"
numbers[0] CHECK numbers[0] == "hello"
numbers[0] CHECK numbers == ["hello", 2, 3, 4, 5]
request.test CHECK request.test == "hello"
request.test CHECK request["test"] == "hello"
numbers[$NUMBER] CHECK numbers[3] == "hello"
request.body CHECK request.body == "hello"
$NUMBER == 4 ? yes : no CHECK no == "hello"
$NUMBER == 3 ? yes : no CHECK yes == "hello"