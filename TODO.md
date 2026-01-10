/*
## 1. **Basic Class Definition (Foundation)**
```java
// Test: Jump to class declaration
public class UserController {  // ← Target
    public void createUser() {}
}

// Usage
UserController controller = new UserController();  // ← Click "UserController"
controller.createUser();                           // ← Click "createUser"
```

**Why**: Most common use case. Tests basic symbol resolution.

## 2. **Method with Parameters**
```java
// Test: Jump to method with correct parameter signature
public class Calculator {
    public int add(int a, int b) {  // ← Target
        return a + b;
    }

    public double add(double a, double b) {
        return a + b;
    }
}

// Usage
Calculator calc = new Calculator();
calc.add(5, 10);  // ← Click "add" (should jump to int version)
```

**Why**: Tests overload resolution based on parameter types.

## 3. **Interface Implementation**
```java
// Test: Jump to interface method from implementation
interface Repository<T> {  // ← Target (when clicking interface name)
    T findById(String id); // ← Target (when clicking method name)
}

class UserRepository implements Repository<User> {
    @Override
    public User findById(String id) {  // Should jump to interface method
        return null;
    }
}

// Usage
Repository<User> repo = new UserRepository();
repo.findById("123");  // ← Click "findById"
```

**Why**: Tests inheritance hierarchy traversal.

## 4. **Static Method/Field**
```java
// Test: Jump to static members
public class MathUtils {
    public static final double PI = 3.14159;  // ← Target

    public static int max(int a, int b) {     // ← Target
        return a > b ? a : b;
    }
}

// Usage
double circle = MathUtils.PI * radius;  // ← Click "PI"
int bigger = MathUtils.max(5, 10);      // ← Click "max"
```

**Why**: Tests static member resolution.

## 5. **Constructor**
```java
// Test: Jump to constructor
public class User {
    private String name;

    public User(String name) {  // ← Target
        this.name = name;
    }
}

// Usage
User user = new User("Alice");  // ← Click "User" or "new User"
```

**Why**: Tests constructor resolution (special case of method).

## 6. **Inner/Nested Class**
```java
// Test: Jump to inner class
public class Outer {
    public class Inner {  // ← Target
        public void innerMethod() {}
    }

    void test() {
        Inner inner = new Inner();  // ← Click "Inner"
        inner.innerMethod();        // ← Click "innerMethod"
    }
}
```

**Why**: Tests scoped symbol resolution within parent class.

## 7. **External Library/Dependency**
```java
// Test: Jump to library class (requires indexing)
import java.util.List;  // ← Should be able to jump to JDK source
import org.springframework.stereotype.Service;

@Service  // ← Click "@Service" (should jump to Spring annotation)
public class UserService {
    private List<String> users;  // ← Click "List" (should jump to JDK)
}
```

**Why**: Tests dependency indexing and external symbol resolution.

## 8. **Getter/Setter (Java Bean)**
```java
// Test: Jump to field from getter/setter
public class Person {
    private String name;  // ← Target (when clicking field)

    public String getName() {  // ← Click "getName"
        return name;
    }

    public void setName(String name) {  // ← Click "setName"
        this.name = name;
    }
}

// Usage
Person p = new Person();
p.setName("Bob");  // ← Click "setName"
String n = p.getName();  // ← Click "getName"
```

**Why**: Tests property accessor resolution.

## 9. **Generic Type**
```java
// Test: Jump to generic class/interface
public class Response<T> {  // ← Target
    private T data;
    public T getData() { return data; }
}

// Usage
Response<User> response = new Response<>();  // ← Click "Response"
User user = response.getData();              // ← Click "getData"
```

**Why**: Tests type parameter resolution.

## **Bonus: Groovy Dynamic Property**
```groovy
// Test: Jump to property with dynamic getter
class User {
    String name  // ← Target

    String getName() {  // ← Alternative target
        return name?.toUpperCase()
    }
}

// Usage
def user = new User(name: "alice")
println user.name  // ← Click "name"
```
*/
